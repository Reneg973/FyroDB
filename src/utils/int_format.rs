pub(crate) const DIGITS100: &[u8; 200] = b"0001020304050607080910111213141516171819\
2021222324252627282930313233343536373839\
4041424344454647484950515253545556575859\
6061626364656667686970717273747576777879\
8081828384858687888990919293949596979899";

#[inline(always)]
fn write_u64_digits(digits: &mut [u8; 20], mut n: u64) -> usize {
   let mut d_pos = digits.len();

   while n >= 100 {
      let rem = ((n % 100) * 2) as usize;
      n /= 100;
      d_pos -= 2;
      digits[d_pos] = DIGITS100[rem];
      digits[d_pos + 1] = DIGITS100[rem + 1];
   }

   if n >= 10 {
      let rem = (n * 2) as usize;
      d_pos -= 2;
      digits[d_pos] = DIGITS100[rem];
      digits[d_pos + 1] = DIGITS100[rem + 1];
   } else {
      d_pos -= 1;
      digits[d_pos] = b'0' + n as u8;
   }

   d_pos
}

#[inline(always)]
pub fn write_u64_to(digits: &mut [u8; 20], n: u64) -> &str {
   let d_pos = write_u64_digits(digits, n);
   unsafe { std::str::from_utf8_unchecked(&digits[d_pos..]) }
}

#[inline(always)]
pub fn write_usize_to(digits: &mut [u8; 20], n: usize) -> &str {
   write_u64_to(digits, n as u64)
}

#[inline(always)]
pub fn write_i64_to(digits: &mut [u8; 20], n: i64) -> &str {
   let mut d_pos = write_u64_digits(digits, n.unsigned_abs());

   if n < 0 {
      d_pos -= 1;
      digits[d_pos] = b'-';
   }

   unsafe { std::str::from_utf8_unchecked(&digits[d_pos..]) }
}

#[inline(always)]
pub fn write_usize(out: &mut Vec<u8>, n: usize) {
   if n < 10 {
      out.push(b'0' + n as u8);
      return;
   }
   if n < 100 {
      out.extend(DIGITS100.iter().skip(n * 2).take(2));
      return;
   }
   let mut digits = [0u8; 20];
   let s = write_usize_to(&mut digits, n);
   let d_len = s.len();
   let old_len = out.len();
   if out.capacity() - old_len < d_len {
      out.reserve(d_len);
   }
   unsafe {
      std::ptr::copy_nonoverlapping(s.as_ptr(), out.as_mut_ptr().add(old_len), d_len);
      out.set_len(old_len + d_len);
   }
}

#[inline(always)]
pub fn write_i64(out: &mut Vec<u8>, n: i64) {
   if n >= 0 {
      write_usize(out, n as usize);
      return;
   }

   let mut digits = [0u8; 20];
   let s = crate::utils::int_format::write_i64_to(&mut digits, n);
   out.extend_from_slice(s.as_bytes());
}
