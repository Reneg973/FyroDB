pub(crate) mod mio_worker;
mod session;
pub(crate) mod stream;

use crate::bus::InvalidationBus;
use crate::cache::DbCacheManager;
use crate::config::{DbCacheConfig, DbProxyMode};
use crate::pool::{PoolLimits, UpstreamPool};
use crate::proxy::stream::ClientStream;
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

pub(crate) const CONTEXT_PROBE: &str = "SELECT current_setting('search_path', true), current_user, \
   current_setting('is_superuser', true), current_setting('row_security', true), \
   current_setting('default_transaction_isolation', true), \
   current_setting('timezone', true), current_setting('datestyle', true), \
   current_setting('intervalstyle', true), current_setting('extra_float_digits', true), \
   current_setting('client_encoding', true), current_setting('lc_numeric', true), \
   current_setting('default_text_search_config', true), \
   current_setting('bytea_output', true), \
   current_setting('standard_conforming_strings', true), \
   current_setting('server_encoding', true), current_database()";

pub(crate) const CATALOG_FUNCTIONS_SQL: &str = "SELECT n.nspname, p.proname FROM pg_proc p \
   JOIN pg_namespace n ON n.oid = p.pronamespace \
   WHERE p.provolatile IN ('s', 'v')";

pub(crate) const CATALOG_VIEWS_SQL: &str = "SELECT v.relname, pg_get_viewdef(v.oid), \
   d.refobjid::regclass::text FROM pg_rewrite r \
   JOIN pg_class v ON v.oid = r.ev_class AND v.relkind IN ('v', 'm') \
   JOIN pg_depend d ON d.objid = r.oid AND d.classid = 'pg_rewrite'::regclass \
      AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'n' \
   WHERE r.rulename = '_RETURN'";

struct ProxyShared {
   pool: Arc<UpstreamPool>,
   bus: Option<Arc<InvalidationBus>>,
}

fn shared_state(config: &DbCacheConfig, cache_mgr: &Arc<DbCacheManager>) -> ProxyShared {
   let pool = Arc::new(UpstreamPool::new(PoolLimits {
      max_idle: config.pool_max_idle,
      idle_timeout: Duration::from_millis(config.pool_idle_timeout_ms),
   }));
   let bus = if config.mode == DbProxyMode::Dual {
      match config.bus_path.as_deref() {
         Some(path) => {
            match crate::bus::start(path, Arc::clone(cache_mgr), config.key_prefix.clone()) {
               Ok(bus) => Some(bus),
               Err(error) => {
                  eprintln!("fyrodb-dbproxy: invalidation bus unavailable ({path}): {error}");
                  None
               }
            }
         }
         None => None,
      }
   } else {
      None
   };
   ProxyShared { pool, bus }
}

fn spawn_janitor(
   config: &DbCacheConfig,
   pool: &Arc<UpstreamPool>,
   cache_mgr: &Arc<DbCacheManager>,
) {
   let janitor_pool = Arc::clone(pool);
   let janitor_mgr = Arc::clone(cache_mgr);
   let janitor_prefix = config.key_prefix.clone();
   let interval = Duration::from_millis(if config.pool_max_idle > 0 {
      (config.pool_idle_timeout_ms / 2).clamp(1_000, 5_000)
   } else {
      1_000
   });
   let _ = thread::Builder::new()
      .name("fyrodb-dbjanitor".into())
      .stack_size(128 * 1024)
      .spawn(move || {
         loop {
            thread::sleep(interval);
            if janitor_mgr.prune_due() {
               janitor_mgr.prune_expired(&janitor_prefix);
            }
            if janitor_pool.idle_count() > 0 {
               janitor_pool.prune();
               janitor_mgr.metrics().pool_idle.store(
                  janitor_pool.idle_count() as u64,
                  std::sync::atomic::Ordering::Relaxed,
               );
            }
         }
      });
}

fn spawn_client(
   client: ClientStream,
   config: DbCacheConfig,
   mgr: Arc<DbCacheManager>,
   pool: Arc<UpstreamPool>,
   bus: Option<Arc<InvalidationBus>>,
) {
   if let Err(e) = thread::Builder::new()
      .name("fyrodb-dbconn".into())
      .stack_size(session::CLIENT_STACK)
      .spawn(move || {
         if let Err(e) = session::handle_client(client, config, mgr, pool, bus)
            && e.kind() != io::ErrorKind::UnexpectedEof
         {
            eprintln!("fyrodb-dbproxy client error: {e}");
         }
      })
   {
      eprintln!("fyrodb-dbproxy: spawn failed: {e}");
   }
}

// Refuses to steal a live socket and clears a stale socket file left behind by
// a previous run, mirroring the invalidation bus broker behavior.
#[cfg(all(unix, feature = "unix-sockets"))]
fn prepare_unix_socket(path: &str) -> io::Result<()> {
   use std::os::unix::net::UnixStream;
   if let Some(parent) = std::path::Path::new(path).parent() {
      std::fs::create_dir_all(parent)?;
   }
   if UnixStream::connect(path).is_ok() {
      return Err(io::Error::new(
         io::ErrorKind::AddrInUse,
         format!("unix socket already in use: {path}"),
      ));
   }
   let _ = std::fs::remove_file(path);
   Ok(())
}

// Spawns the background TCP listener for PostgreSQL wire proxy caching.
// listen_addr is "host:port" for TCP or "unix:<dir>" for a Unix socket, in
// which case the listener binds <dir>/.s.PGSQL.<listen_port>.
pub fn start_proxy_listener(
   config: DbCacheConfig,
   cache_mgr: Arc<DbCacheManager>,
) -> io::Result<thread::JoinHandle<()>> {
   if config.unix_socket_dir().is_some() {
      return start_proxy_listener_unix(config, cache_mgr);
   }

   let listener = TcpListener::bind(&config.listen_addr)?;
   let listen_addr = config.listen_addr.clone();
   let shared = shared_state(&config, &cache_mgr);
   spawn_janitor(&config, &shared.pool, &cache_mgr);

   let config_c = config.clone();
   let mgr_c = Arc::clone(&cache_mgr);
   let pool_c = Arc::clone(&shared.pool);
   let bus_c = shared.bus.clone();

   thread::Builder::new()
      .name("fyrodb-dbproxy".into())
      .stack_size(256 * 1024)
      .spawn(move || {
         for stream in listener.incoming() {
            match stream {
               Ok(client) => spawn_client(
                  ClientStream::Tcp(client),
                  config_c.clone(),
                  Arc::clone(&mgr_c),
                  Arc::clone(&pool_c),
                  bus_c.clone(),
               ),
               Err(e) => {
                  eprintln!("fyrodb-dbproxy accept error on {listen_addr}: {e}");
               }
            }
         }
      })
}

#[cfg(all(unix, feature = "unix-sockets"))]
fn start_proxy_listener_unix(
   config: DbCacheConfig,
   cache_mgr: Arc<DbCacheManager>,
) -> io::Result<thread::JoinHandle<()>> {
   let sock_path = config
      .unix_socket_file()
      .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid unix listen address"))?;
   prepare_unix_socket(&sock_path)?;
   let listener = std::os::unix::net::UnixListener::bind(&sock_path)?;

   let shared = shared_state(&config, &cache_mgr);
   spawn_janitor(&config, &shared.pool, &cache_mgr);

   let config_c = config.clone();
   let mgr_c = Arc::clone(&cache_mgr);
   let pool_c = Arc::clone(&shared.pool);
   let bus_c = shared.bus.clone();

   thread::Builder::new()
      .name("fyrodb-dbproxy-unix".into())
      .stack_size(256 * 1024)
      .spawn(move || {
         for stream in listener.incoming() {
            match stream {
               Ok(client) => spawn_client(
                  ClientStream::Unix(client),
                  config_c.clone(),
                  Arc::clone(&mgr_c),
                  Arc::clone(&pool_c),
                  bus_c.clone(),
               ),
               Err(e) => {
                  eprintln!("fyrodb-dbproxy unix accept error on {sock_path}: {e}");
               }
            }
         }
      })
}

#[cfg(not(all(unix, feature = "unix-sockets")))]
fn start_proxy_listener_unix(
   _config: DbCacheConfig,
   _cache_mgr: Arc<DbCacheManager>,
) -> io::Result<thread::JoinHandle<()>> {
   Err(io::Error::new(
      io::ErrorKind::Unsupported,
      "unix socket listen addresses require a unix platform and the 'unix-sockets' feature",
   ))
}

// Spawns N mio worker threads, each owning a SO_REUSEPORT listener on the
// same address. FyroDB's event loop is completely unaffected. Unix socket
// addresses bind <dir>/.s.PGSQL.<listen_port> on a single worker — unix
// sockets cannot share a path via SO_REUSEPORT.
pub fn start_proxy_listener_mio(
   config: DbCacheConfig,
   cache_mgr: Arc<DbCacheManager>,
) -> io::Result<Vec<thread::JoinHandle<()>>> {
   use mio_worker::{bind_tcp_listener, bind_unix_listener};

   let unix = config.unix_socket_file();
   let n_workers = if unix.is_some() {
      1
   } else if config.proxy_workers == 0 {
      std::thread::available_parallelism()
         .map(|n| n.get())
         .unwrap_or(4)
         .min(16)
   } else {
      config.proxy_workers
   };

   let mut listeners = Vec::with_capacity(n_workers);
   if let Some(sock_path) = unix {
      listeners.push(bind_unix_listener(&sock_path)?);
   } else {
      let addr: std::net::SocketAddr = config.listen_addr.parse().map_err(|e| {
         io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid listen addr: {e}"),
         )
      })?;
      for _ in 0..n_workers {
         listeners.push(bind_tcp_listener(addr)?);
      }
   }

   let shared = shared_state(&config, &cache_mgr);

   // Shared blocking sub-pool for connections that need upstream access.
   let blocking_threads = n_workers.max(2);
   let blocking_pool = mio_worker::BlockingPool::start(blocking_threads);

   let mut handles = Vec::with_capacity(n_workers);
   for (i, listener) in listeners.into_iter().enumerate() {
      let cfg = Arc::new(config.clone());
      let mgr = Arc::clone(&cache_mgr);
      let pool_c = Arc::clone(&shared.pool);
      let bus_c = shared.bus.clone();
      let bp = Arc::clone(&blocking_pool);
      let (dtx, drx) = mpsc::sync_channel::<usize>(blocking_threads * 16);

      let handle = thread::Builder::new()
         .name(format!("fyrodb-mio-proxy-{i}"))
         .stack_size(256 * 1024)
         .spawn(move || {
            mio_worker::run_proxy_worker(listener, mgr, cfg, pool_c, bus_c, bp, drx, dtx);
         })?;
      handles.push(handle);
   }
   Ok(handles)
}
