// SPDX-License-Identifier: GPL-2.0-only
// Offline only: calls the compiled C descriptor/helper, never a device or kernel.
#[allow(dead_code)]
mod reference {
    include!("reference-model.rs");
    pub fn need(w:u32,h:u32,c:u32,d:u32,t:u32)->[u32;10] { hevc(w,h,c,d,t) }
    pub fn envelope(w:u32,h:u32)->[u32;10] { candidate(w,h) }
}
unsafe extern "C" {
 fn candidate_size(slot:u32,w:u32,h:u32)->usize;
 fn original_381_size(slot:u32,w:u32,h:u32)->usize;
 fn unchanged_381_size(slot:u32,w:u32,h:u32)->usize;
 fn unchanged_383_size(slot:u32,w:u32,h:u32)->usize;
 fn original_383_size(slot:u32,w:u32,h:u32)->usize;
 fn original_383_slots()->u32;
 fn candidate_slots()->u32;
 fn candidate_axis(slot:u32)->u32;
 fn candidate_alignment(slot:u32)->u32;
}
fn sizes(w:u32,h:u32)->[u32;10] { std::array::from_fn(|i|unsafe{candidate_size(i as u32,w,h) as u32}) }
fn sweep()->u64 {
 let mut count=0;
 for w in (64..=4096u32).step_by(16) { for h in (64..=2304u32).step_by(16) {
  let have=sizes(w,h); assert_eq!(have,reference::envelope(w,h));
  for i in 0..10 {unsafe{assert_eq!(original_381_size(i,w,h),unchanged_381_size(i,w,h));assert!(have[i as usize] as usize>=original_381_size(i,w,h));}}
  for c in [16,32,64] {for d in [8,10] {for t in 0..=19.min(w.div_ceil(c)-1) {
   let need=reference::need(w,h,c,d,t);
   for i in 0..10 {assert!(have[i]>=need[i],"w={w} h={h} ctu={c} depth={d} cuts={t} slot={i}: {} < {}",have[i],need[i]);}
   count+=1;
  }}}
 }}
 count
}
fn main() {
 let count=sweep();println!("PASS vendor-formula combinations={count}; C-envelope shortfalls=0");
 for (w,h) in [(3840,2160),(4096,2160)] {
  let have=sizes(w,h);let old:[u32;10]=std::array::from_fn(|i|unsafe{original_381_size(i as u32,w,h) as u32});
  println!("{w}x{h}: candidate={have:?}; original={old:?}; total_bytes={} original_total={} additional={}",have.iter().sum::<u32>(),old.iter().sum::<u32>(),have.iter().sum::<u32>()-old.iter().sum::<u32>());
 }
}
#[cfg(test)]mod actual_c_tests {
 use super::*;
 #[test]fn ten_register_slots_preserved(){unsafe{assert_eq!(candidate_slots(),10);for i in 0..10 {assert_eq!(candidate_axis(i),if [2,5,9].contains(&i){1}else{0});assert_eq!(candidate_alignment(i),64);}}}
 #[test]fn unaligned_dimensions_round_up(){assert_eq!(sizes(1919,1079),reference::envelope(1919,1079));assert_eq!(sizes(3856,2176),reference::envelope(3856,2176));}
 #[test]fn camera_and_boundary_cover_refinement(){for w in [3840,4096] {assert!(sizes(w,2160)[0]>=reference::need(w,2160,64,8,1)[0]);}}
 #[test]fn other_variants_keep_zero_alignment(){unsafe{for (w,h) in [(1,1),(1919,1079),(3840,2160),(4096,2304)] {for i in 0..10 {assert_eq!(unchanged_381_size(i,w,h),original_381_size(i,w,h));} for i in 0..original_383_slots() {assert_eq!(unchanged_383_size(i,w,h),original_383_size(i,w,h));}}}}
 #[test]fn exhaustive_vendor_envelope(){assert_eq!(sweep(),3_952_794);}
}
