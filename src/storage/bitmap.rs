use crate::storage::store::Store;
use crate::storage::value::{SmallStr, StoreValue};

impl Store {
   pub fn setbit(&self, key: &str, offset: u64, value: bool) -> Result<u8, &'static str> {
      let byte_idx = (offset / 8) as usize;
      let bit_idx = 7 - (offset % 8) as u8;
      let mask = 1u8 << bit_idx;

      let result = self.data.update_with(key, |val| {
         let is_expired = val.is_expired();
         match val.value.as_string_mut() {
            Some(s) => {
               s.ensure_len(byte_idx + 1);
               let bytes = s.as_bytes_mut();
               let p = bytes.as_mut_ptr();
               if is_expired {
                  unsafe { std::ptr::write_bytes(p, 0, bytes.len()) };
                  val.expires_ms = 0;
               }
               unsafe {
                  let p = p.add(byte_idx);
                  let old = (*p & mask) >> bit_idx;
                  if value {
                     *p |= mask;
                  } else {
                     *p &= !mask;
                  }
                  Ok(old)
               }
            }
            None => Err("WRONGTYPE"),
         }
      });

      match result {
         Some(r) => r,
         None => {
            let mut s = SmallStr::with_capacity(byte_idx + 1);
            let bytes = s.as_bytes_mut();
            bytes[byte_idx] |= mask;
            self.data.insert_if_absent_str(key, StoreValue::string_small(s));
            Ok(0)
         }
      }
   }

   pub fn getbit(&self, key: &str, offset: u64) -> Result<u8, &'static str> {
      match self.data.get_ref(key) {
         None => Ok(0),
         Some(e) if e.is_expired() => Ok(0),
         Some(e) => match e.value.as_string() {
            Some(s) => {
               let byte_idx = (offset / 8) as usize;
               let bit_idx = 7 - (offset % 8) as u8;
               let bytes = s.as_bytes();
               if byte_idx >= bytes.len() {
                  Ok(0)
               } else {
                  Ok((bytes[byte_idx] >> bit_idx) & 1)
               }
            }
            None => Err("WRONGTYPE"),
         },
      }
   }

   pub fn bitcount(
      &self,
      key: &str,
      start: i64,
      end: i64,
      use_bit: bool,
   ) -> Result<usize, &'static str> {
      match self.data.get_ref(key) {
         None => Ok(0),
         Some(e) if e.is_expired() => Ok(0),
         Some(e) => match e.value.as_string() {
            Some(s) => {
               let bytes = s.as_bytes();
               if bytes.is_empty() {
                  return Ok(0);
               }
               if use_bit {
                  let bit_len = bytes.len() * 8;
                  let s_idx = normalize_bit_index(start, bit_len);
                  let e_idx = normalize_bit_index(end, bit_len);
                  if s_idx > e_idx {
                     return Ok(0);
                  }
                  let s_byte = s_idx / 8;
                  let e_byte = e_idx / 8;
                  let count = if s_byte == e_byte {
                     let mask = (0xFF_u8 >> (s_idx % 8)) & (0xFF_u8 << (7 - e_idx % 8));
                     (bytes[s_byte] & mask).count_ones() as usize
                  } else {
                     let head = (bytes[s_byte] & (0xFF_u8 >> (s_idx % 8))).count_ones() as usize;
                     let mid: usize = bytes[s_byte + 1..e_byte]
                        .iter()
                        .map(|b| b.count_ones() as usize)
                        .sum();
                     let tail =
                        (bytes[e_byte] & (0xFF_u8 << (7 - e_idx % 8))).count_ones() as usize;
                     head + mid + tail
                  };
                  Ok(count)
               } else {
                  let len = bytes.len() as i64;
                  let s_idx = if start < 0 {
                     (len + start).max(0)
                  } else {
                     start.min(len)
                  } as usize;
                  let e_idx = if end < 0 {
                     (len + end).max(0)
                  } else {
                     end.min(len - 1)
                  } as usize;
                  if s_idx > e_idx {
                     return Ok(0);
                  }
                  let count = bytes[s_idx..=e_idx]
                     .iter()
                     .map(|b| b.count_ones() as usize)
                     .sum();
                  Ok(count)
               }
            }
            None => Err("WRONGTYPE"),
         },
      }
   }

   pub fn bitpos(
      &self,
      key: &str,
      bit: bool,
      start: i64,
      end: i64,
      has_end: bool,
      use_bit: bool,
   ) -> Result<i64, &'static str> {
      match self.data.get_ref(key) {
         None => Ok(if bit { -1 } else { 0 }),
         Some(e) if e.is_expired() => {
            Ok(if bit { -1 } else { 0 })
         }
         Some(e) => match e.value.as_string() {
            Some(s) => {
               let bytes = s.as_bytes();
               if bytes.is_empty() {
                  return Ok(if bit { -1 } else { 0 });
               }
               if use_bit {
                  let bit_len = bytes.len() * 8;
                  let s_idx = normalize_bit_index(start, bit_len);
                  let e_idx = normalize_bit_index(end, bit_len);
                  if s_idx > e_idx {
                     return Ok(-1);
                  }
                  let s_byte = s_idx / 8;
                  let e_byte = e_idx / 8;
                  let mut lo_mask = 0xFF_u8 >> (s_idx % 8);
                  for byte_i in s_byte..=e_byte.min(bytes.len() - 1) {
                     let hi = if byte_i == e_byte { e_idx % 8 } else { 7 };
                     let mask = lo_mask & (0xFF_u8 << (7 - hi));
                     let target = if bit {
                        bytes[byte_i] & mask
                     } else {
                        !bytes[byte_i] & mask
                     };
                     if target != 0 {
                        return Ok((byte_i * 8 + target.leading_zeros() as usize) as i64);
                     }
                     lo_mask = 0xFF_u8;
                  }
                  Ok(-1)
               } else {
                  let len = bytes.len() as i64;
                  let s_idx = if start < 0 {
                     (len + start).max(0)
                  } else {
                     start.min(len)
                  } as usize;
                  let e_idx = if end < 0 {
                     (len + end).max(0) as usize
                  } else if has_end {
                     end.min(len - 1) as usize
                  } else {
                     bytes.len() - 1
                  };
                  if s_idx > e_idx {
                     return Ok(-1);
                  }
                  for (byte_idx, &b) in bytes.iter().enumerate().take(e_idx + 1).skip(s_idx) {
                     let target = if bit { b } else { !b };
                     if target != 0 {
                        return Ok((byte_idx * 8 + target.leading_zeros() as usize) as i64);
                     }
                  }
                  if !bit && !has_end {
                     Ok((bytes.len() * 8) as i64)
                  } else {
                     Ok(-1)
                  }
               }
            }
            None => Err("WRONGTYPE"),
         },
      }
   }

   #[inline]
   fn combine_with(&self, keys: &[&str], len: usize, init: u8, f: impl Fn(u8, u8) -> u8) -> Result<Vec<u8>, &'static str> {
      let mut result = vec![init; len];
      keys.iter()
         .filter_map(|&k| self.data.get_ref(k))
         .try_for_each(|e| {
            if let Some(s) = e.value.as_string() {
               result.iter_mut().zip(s.as_bytes()).for_each(|(d, &b)| *d = f(*d, b));
               return Ok(());
            }
            return Err("WRONGTYPE");
         })?;
      Ok(result)
   }

   pub fn bitop(&self, op: BitOp, dest: &str, keys: &[&str]) -> Result<usize, &'static str> {
      // Pre‑allocate a single result buffer sized to the maximum length of any source.
      let max_len = keys.iter()
         .filter_map(|&k| self.data.get_ref(k)?.value.as_string().map(|s| s.len()))
         .max()
         .unwrap_or(0);

      // If no source data, create an empty destination.
      if max_len == 0 {
         self.data.insert(dest.to_string(), StoreValue::string(String::new()));
         return Ok(0);
      }

      let result: Vec<u8> = match op {
         BitOp::And => self.combine_with(keys, max_len, 0xFF, |d, s| d & s)?,
         BitOp::Or => self.combine_with(keys, max_len, 0, |d, s| d | s)?,
         BitOp::Xor => self.combine_with(keys, max_len, 0, |d, s| d ^ s)?,
         BitOp::Not => {
            if keys.len() != 1 {
               return Err("BITOP NOT requires one source key");
            }
            let src = self.data.get_ref(keys[0]).ok_or("WRONGTYPE")?;
            let s = src.value.as_string().ok_or("WRONGTYPE")?;
            // Built directly from the source: no pre-filled buffer needed.
            s.as_bytes().iter().map(|b| !b).collect()
         }
      };

      self.data.insert(
         dest.to_string(),
         StoreValue::string(unsafe { String::from_utf8_unchecked(result) }),
      );
      Ok(max_len)
   }

   pub fn bf_get(
      &self,
      key: &str,
      offset: u64,
      bits: u32,
      signed: bool,
   ) -> Result<i64, &'static str> {
      if bits == 0 {
         return Ok(0);
      }
      if bits > 64 {
         return Err("invalid bitfield type");
      }
      match self.data.get_ref(key) {
         None => Ok(0),
         Some(e) if e.is_expired() => Ok(0),
         Some(e) => match e.value.as_string() {
            Some(s) => {
               let bytes = s.as_bytes();
               let start_bit = offset;
               let end_bit = offset + bits as u64;
               let start_byte = (start_bit / 8) as usize;
               let end_byte = end_bit.div_ceil(8) as usize;
               let mut val: u64 = 0;

               for b_idx in start_byte..end_byte {
                  let byte_val = if b_idx < bytes.len() { bytes[b_idx] } else { 0 };
                  let b_start = start_bit.max(b_idx as u64 * 8);
                  let b_end = end_bit.min(b_idx as u64 * 8 + 8);
                  let num_bits = (b_end - b_start) as u32;
                  let shift_right = (b_idx as u64 * 8 + 8 - b_end) as u32;
                  let mask = ((1u16 << num_bits) - 1) as u8;
                  let extracted_bits = ((byte_val >> shift_right) & mask) as u64;
                  let shift_left = (end_bit - b_end) as u32;
                  val |= extracted_bits << shift_left;
               }

               if signed && bits < 64 && (val >> (bits - 1)) & 1 == 1 {
                  Ok((val | (!0u64 << bits)) as i64)
               } else {
                  Ok(val as i64)
               }
            }
            None => Err("WRONGTYPE"),
         },
      }
   }

   pub fn bf_set(&self, key: &str, offset: u64, bits: u32, value: i64) -> Result<(), &'static str> {
      if bits == 0 {
         return Ok(());
      }
      if bits > 64 {
         return Err("invalid bitfield type");
      }

      let last_byte_idx = ((offset + bits as u64 - 1) / 8) as usize;
      let val = value as u64;

      let write_bits = |bytes: &mut [u8]| {
         let start_bit = offset;
         let end_bit = offset + bits as u64;
         let start_byte = (start_bit / 8) as usize;
         let end_byte = end_bit.div_ceil(8) as usize;

         for (offset_idx, byte) in bytes[start_byte..end_byte].iter_mut().enumerate() {
            let b_idx = start_byte + offset_idx;
            let b_start = start_bit.max(b_idx as u64 * 8);
            let b_end = end_bit.min(b_idx as u64 * 8 + 8);
            let num_bits = (b_end - b_start) as u32;
            let val_shift_right = (end_bit - b_end) as u32;
            let bits_to_write =
               ((val >> val_shift_right) & (((1u16 << num_bits) - 1) as u64)) as u8;
            let byte_shift_left = (b_idx as u64 * 8 + 8 - b_end) as u32;
            let shifted_bits = bits_to_write << byte_shift_left;
            let mask = (((1u16 << num_bits) - 1) as u8) << byte_shift_left;
            *byte = (*byte & !mask) | shifted_bits;
         }
      };

      let result = self.data.update_with(key, |v| {
         let is_expired = v.is_expired();
         match v.value.as_string_mut() {
            Some(s) => {
               s.ensure_len(last_byte_idx + 1);
               let bytes = s.as_bytes_mut();
               if is_expired {
                  bytes.fill(0);
               }
               write_bits(bytes);
               Ok(())
            }
            None => Err("WRONGTYPE"),
         }
      });

      match result {
         Some(r) => r,
         None => {
            let mut s = SmallStr::with_capacity(last_byte_idx + 1);
            write_bits(s.as_bytes_mut());
            self.data.insert(
               key.to_string(),
               StoreValue::string_small(s)
            );
            Ok(())
         }
      }
   }
}

#[derive(Clone, Copy)]
pub enum BitOp {
   And,
   Or,
   Xor,
   Not,
}

#[inline]
fn normalize_bit_index(idx: i64, bit_len: usize) -> usize {
   if idx < 0 {
      (bit_len as i64 + idx).max(0) as usize
   } else {
      (idx as usize).min(bit_len.saturating_sub(1))
   }
}
