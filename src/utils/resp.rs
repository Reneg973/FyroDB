use crate::utils::int_format::{DIGITS100, write_i64, write_usize};

#[inline(always)]
pub fn fast_parse_f64_bytes(b: &[u8]) -> Option<f64> {
   if b.is_empty() {
      return None;
   }
   let mut i = 0;
   let neg = if b[0] == b'-' {
      i += 1;
      true
   } else if b[0] == b'+' {
      i += 1;
      false
   } else {
      false
   };
   if i == b.len() {
      return None;
   }
   let mut int_part: u64 = 0;
   let mut has_int = false;
   while i < b.len() && b[i].is_ascii_digit() {
      has_int = true;
      int_part = int_part.wrapping_mul(10).wrapping_add((b[i] - b'0') as u64);
      i += 1;
   }
   if i == b.len() && has_int {
      let f = int_part as f64;
      return Some(if neg { -f } else { f });
   }
   if i < b.len() && b[i] == b'.' {
      i += 1;
      let mut frac_part: u64 = 0;
      let mut frac_div: f64 = 1.0;
      let mut has_frac = false;
      while i < b.len() && b[i].is_ascii_digit() && frac_div < 1e15 {
         has_frac = true;
         frac_part = frac_part * 10 + (b[i] - b'0') as u64;
         frac_div *= 10.0;
         i += 1;
      }
      if i == b.len() && (has_int || has_frac) {
         let f = int_part as f64 + (frac_part as f64 / frac_div);
         return Some(if neg { -f } else { f });
      }
   }
   std::str::from_utf8(b)
      .ok()
      .and_then(|s| s.parse::<f64>().ok())
}

#[inline(always)]
pub fn fast_parse_f64(s: &str) -> Option<f64> {
   fast_parse_f64_bytes(s.as_bytes())
}

pub const OK: &[u8] = b"+OK\r\n";
pub const NIL: &[u8] = b"$-1\r\n";
pub const PONG: &[u8] = b"+PONG\r\n";

pub const INT_CACHE: [&[u8]; 10] = [
   b":0\r\n", b":1\r\n", b":2\r\n", b":3\r\n", b":4\r\n", b":5\r\n", b":6\r\n", b":7\r\n",
   b":8\r\n", b":9\r\n",
];

const RESP_BULK_HEADERS: [u32; 10] = [
   u32::from_ne_bytes(*b"$0\r\n"),
   u32::from_ne_bytes(*b"$1\r\n"),
   u32::from_ne_bytes(*b"$2\r\n"),
   u32::from_ne_bytes(*b"$3\r\n"),
   u32::from_ne_bytes(*b"$4\r\n"),
   u32::from_ne_bytes(*b"$5\r\n"),
   u32::from_ne_bytes(*b"$6\r\n"),
   u32::from_ne_bytes(*b"$7\r\n"),
   u32::from_ne_bytes(*b"$8\r\n"),
   u32::from_ne_bytes(*b"$9\r\n"),
];
const CRLF_U16: u16 = u16::from_ne_bytes(*b"\r\n");

#[inline(always)]
pub(crate) fn is_crlf_at(p: *const u8, n: usize) -> bool {
   unsafe { p.add(n).cast::<u16>().read_unaligned() == CRLF_U16 }
}

#[inline(always)]
pub(crate) fn write_crlf_at(p: *mut u8, n: usize) {
   unsafe {
      p.add(n).cast::<u16>().write_unaligned(CRLF_U16);
   }
}

#[inline(always)]
pub fn format_usize_digits(mut n: usize, buf: &mut [u8]) -> &[u8] {
   if n == 0 {
      buf[0] = b'0';
      return &buf[..1];
   }
   let mut i = buf.len();
   while n > 0 {
      i -= 1;
      buf[i] = b'0' + (n % 10) as u8;
      n /= 10;
   }
   &buf[i..]
}

/// # Safety
///
/// Caller must ensure `dst` has enough capacity to hold the formatted RESP bulk header, payload, and CRLF.
#[inline(always)]
pub unsafe fn write_bulk_raw(dst: *mut u8, s: &[u8]) -> usize {
   unsafe {
      let len = s.len();
      if len < 10 {
         let total = 6 + len;
         let header = 0x0A0D_3024u32 | ((len as u32) << 8);
         dst.cast::<u32>().write_unaligned(header);
         std::ptr::copy_nonoverlapping(s.as_ptr(), dst.add(4), len);
         write_crlf_at(dst, 4 + len);
         total
      } else if len < 100 {
         let total = 7 + len;
         let d1 = b'0' + (len / 10) as u8;
         let d2 = b'0' + (len % 10) as u8;
         let header = 0x0D00_0024u32 | ((d1 as u32) << 8) | ((d2 as u32) << 16);
         dst.cast::<u32>().write_unaligned(header);
         *dst.add(4) = b'\n';
         std::ptr::copy_nonoverlapping(s.as_ptr(), dst.add(5), len);
         write_crlf_at(dst, 5 + len);
         total
      } else {
         let mut p = dst;
         *p = b'$';
         p = p.add(1);
         let mut num_buf = [0u8; 20];
         let num_str = format_usize_digits(len, &mut num_buf);
         std::ptr::copy_nonoverlapping(num_str.as_ptr(), p, num_str.len());
         p = p.add(num_str.len());
         write_crlf_at(p, 0);
         p = p.add(2);
         std::ptr::copy_nonoverlapping(s.as_ptr(), p, len);
         p = p.add(len);
         p.cast::<u16>().write_unaligned(CRLF_U16);
         p = p.add(2);
         p.offset_from(dst) as usize
      }
   }
}

#[inline]
pub fn write_ok(out: &mut Vec<u8>) {
   out.extend_from_slice(OK);
}

#[inline]
pub fn write_pong(out: &mut Vec<u8>) {
   out.extend_from_slice(PONG);
}

#[inline]
pub fn write_nil(out: &mut Vec<u8>) {
   out.extend_from_slice(NIL);
}

#[inline]
pub fn write_bulk(out: &mut Vec<u8>, s: &str) {
   write_bulk_bytes(out, s.as_bytes());
}

#[inline]
pub fn write_float(out: &mut Vec<u8>, f: f64) {
   let mut buffer = ryu::Buffer::new();
   write_bulk(out, buffer.format(f));
}

#[cold]
#[inline(never)]
fn reserve_resp(out: &mut Vec<u8>, total: usize) {
   out.reserve(total);
}

#[inline(always)]
pub fn write_bulk_bytes(out: &mut Vec<u8>, s: &[u8]) {
   let len = s.len();
   if len < 10 {
      let old_len = out.len();
      let total = 4 + len + 2;
      if out.capacity() - old_len < total {
         reserve_resp(out, total);
      }
      unsafe {
         let p = out.as_mut_ptr().add(old_len);
         p.cast::<u32>()
            .write_unaligned(*RESP_BULK_HEADERS.get_unchecked(len));
         std::ptr::copy_nonoverlapping(s.as_ptr(), p.add(4), len);
         write_crlf_at(p, 4 + len);
         out.set_len(old_len + total);
      }
      return;
   }
   if len < 100 {
      let old_len = out.len();
      let total = 5 + len + 2;
      if out.capacity() - old_len < total {
         reserve_resp(out, total);
      }
      unsafe {
         let p = out.as_mut_ptr().add(old_len);
         *p = b'$';
         let d_idx = len * 2;
         std::ptr::copy_nonoverlapping(DIGITS100.as_ptr().add(d_idx), p.add(1), 2);
         write_crlf_at(p, 3);
         std::ptr::copy_nonoverlapping(s.as_ptr(), p.add(5), len);
         write_crlf_at(p, 5 + len);
         out.set_len(old_len + total);
      }
      return;
   }
   out.reserve(1 + 10 + 2 + len + 2);
   out.push(b'$');
   write_usize(out, len);
   out.extend_from_slice(b"\r\n");
   out.extend_from_slice(s);
   out.extend_from_slice(b"\r\n");
}

#[inline(always)]
pub fn write_integer(out: &mut Vec<u8>, n: i64) {
   if (0..10).contains(&n) {
      out.extend_from_slice(INT_CACHE[n as usize]);
      return;
   }
   out.push(b':');
   write_i64(out, n);
   out.extend_from_slice(b"\r\n");
}

#[inline]
pub fn write_boolean(out: &mut Vec<u8>, b: bool) {
   out.extend_from_slice(INT_CACHE[b as usize]);
}

#[inline]
pub fn write_opt_bulk(out: &mut Vec<u8>, v: Option<String>) {
   match v {
      Some(s) => write_bulk(out, &s),
      None => write_nil(out),
   }
}

#[inline]
pub fn write_opt_bulk_ref(out: &mut Vec<u8>, v: Option<&str>) {
   match v {
      Some(s) => write_bulk(out, s),
      None => write_nil(out),
   }
}

#[inline]
pub fn write_array_header(out: &mut Vec<u8>, len: usize) {
   out.push(b'*');
   write_usize(out, len);
   out.extend_from_slice(b"\r\n");
}

#[inline]
pub fn write_array(out: &mut Vec<u8>, items: &[String]) {
   write_array_header(out, items.len());
   for item in items {
      write_bulk(out, item);
   }
}

#[inline]
pub fn write_opt_array(out: &mut Vec<u8>, items: &[Option<String>]) {
   write_array_header(out, items.len());
   for item in items {
      match item {
         Some(s) => write_bulk(out, s),
         None => write_nil(out),
      }
   }
}

#[inline]
pub fn write_err(out: &mut Vec<u8>, msg: &str) {
   out.reserve(msg.len() + 7);
   out.extend_from_slice(b"-ERR ");
   out.extend_from_slice(msg.as_bytes());
   out.extend_from_slice(b"\r\n");
}

#[inline]
pub fn write_store_err(out: &mut Vec<u8>, err: &str) {
   if err == "WRONGTYPE" {
      write_wrong_type(out);
      return;
   }
   let code = err.split(' ').next().unwrap_or("");
   if code.len() >= 3 && err.len() > code.len() && code.bytes().all(|b| b.is_ascii_uppercase()) {
      out.push(b'-');
      out.extend_from_slice(err.as_bytes());
      out.extend_from_slice(b"\r\n");
   } else {
      write_err(out, err);
   }
}

#[inline]
pub fn write_wrong_type(out: &mut Vec<u8>) {
   out.extend_from_slice(b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n");
}

#[inline]
pub fn write_wrong_args(out: &mut Vec<u8>, cmd: &str) {
   out.reserve(cmd.len() + 47);
   out.extend_from_slice(b"-ERR wrong number of arguments for '");
   out.extend_from_slice(cmd.as_bytes());
   out.extend_from_slice(b"' command\r\n");
}

#[inline]
pub fn write_simple(out: &mut Vec<u8>, s: &str) {
   out.reserve(s.len() + 3);
   out.push(b'+');
   out.extend_from_slice(s.as_bytes());
   out.extend_from_slice(b"\r\n");
}
