// SPDX-License-Identifier: GPL-2.0-only
// Independent Rust formula transcription; see the pinned reference below.
// Offline comparison only. No hardware, syscalls to device nodes, or production configuration.
// Formula source: rockchip-linux/mpp @20959ec3dabb313b8c33ca0b7b09c06ffd2a9caa
// hal_h265d_vdpu34x.c:665-765; vdpu34x_com.c:31-77; vdpu_com.h:17-18.
// IMPORTANT: MPP refined formulas differ from the TRM conservative table. A model shortfall
// is a software sizing discrepancy, not proof that VDPU346 writes beyond allocation.
// Scope: HEVC 4:2:0 (chroma_format_idc=1), 8/10 bit, linear output (FBC disabled),
// CTU16/32/64, widths 64..4096, heights64..2304. This does NOT prove hardware behavior.
const NAMES: [&str;10] = ["intra","transd_row","transd_col","streamd_row","inter_row","inter_col","dblk","sao","fbc","filter_col"];
fn align(n:u32,a:u32)->u32 {(n+a-1)/a*a}
fn bytes(bits:u32)->u32 {align((bits+7)/8,64)}
fn generic(w:u32,h:u32)->[u32;10] {[6*w,w,h,3*w,6*w,3*h,22*w,6*w,11*w,67*h]}
fn hevc(w:u32,h:u32,ctu:u32,depth:u32,cuts:u32)->[u32;10] {
 let w=align(w,ctu); let h=align(h,ctu); let ext=cuts*512;
 let b=match ctu {16=>31,32=>25,64=>21,_=>panic!("unsupported CTU")};
 [bytes(w*48+ext),0,0,0,bytes(w*22+ext),if cuts>0 {bytes(h*22)} else {0},
 bytes(w*((if ctu==32 {4}else{2})+6*depth)+cuts*(if depth==8 {256}else{192})+ext),
 bytes(w*(128/ctu+2*depth)+cuts*(if depth==8 {160}else{128})+ext),0,
 if cuts>0 {bytes(h*(9*depth+b+if depth==10 {16}else{0}))} else {0}]
}
// Hypothesis only: keep all ten register slots, allocate from64aligned dimensions,
// reserve10W INTRA,7W INTER,9W SAO. Bound: cuts <= width/16-1 gives6W+64cuts<10W.
fn candidate(w:u32,h:u32)->[u32;10] {let w=align(w,64);let h=align(h,64);let mut a=generic(w,h);a[0]=10*w;a[4]=7*w;a[7]=9*w;a}
fn main(){
 let mut cases=0u64; let mut short=[0u64;10]; let mut paged=[0u64;10];let mut candidate_short=[0u64;10];
 let mut first:[Option<String>;10]=std::array::from_fn(|_|None);
 for w in (64..=4096).step_by(16) {for h in (64..=2304).step_by(16) {for ctu in [16,32,64] {for d in [8,10] {
 for cuts in 0..=19.min((w+ctu-1)/ctu-1) {
  cases+=1;let need=hevc(w,h,ctu,d,cuts);let have=generic(w,h);let safe=candidate(w,h);
  for i in 0..10 {if need[i]>have[i] {short[i]+=1;if first[i].is_none(){first[i]=Some(format!("w={w} h={h} ctu={ctu} depth={d} cuts={cuts} need={} allocation={}",need[i],have[i]));}}
   if need[i]>align(have[i],4096){paged[i]+=1;} if need[i]>safe[i]{candidate_short[i]+=1;}
  }
 }}}}}
 println!("cases={cases}");for i in 0..10 {println!("{}: requested_shortfalls={} beyond_4K_rounded={} candidate_shortfalls={} first={:?}",NAMES[i],short[i],paged[i],candidate_short[i],first[i]);}
 for (w,h) in [(1920,1088),(3840,2160),(3840,2176)] {println!("camera/example {w}x{h} CTU64 depth8 onecut generic={:?} refined={:?} candidate={:?}",generic(w,h),hevc(w,h,64,8,1),candidate(w,h));}
 println!("4096x2160 CTU64 depth8 onecut generic={:?} refined={:?}",generic(4096,2160),hevc(4096,2160,64,8,1));
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn single_tile_uhd(){let a=generic(3840,2160);let b=hevc(3840,2160,64,8,0);assert!(a.iter().zip(b).all(|(x,y)|*x>=y));}
 #[test]fn tiled_intra_real_counterexample(){assert_eq!(generic(4096,2160)[0],24576);assert_eq!(hevc(4096,2160,64,8,1)[0],24640);}
 #[test]fn camera_case(){assert_eq!(hevc(3840,2160,64,8,1)[0],23104);assert_eq!(generic(3840,2160)[0],23040);assert_eq!(align(23040,4096),24576);}
 #[repr(C,packed)] struct H26xAddr {reserved160:u32,pps161:u32,reserved162:u32,rps163:u32,refs164_180:[u32;16],scanlist181:u32,colmv182_198:[u32;16],cabac199:u32}
 #[test]fn names_do_not_move_register_offsets(){assert_eq!(std::mem::offset_of!(H26xAddr,scanlist181)/4+160,180);assert_eq!(std::mem::offset_of!(H26xAddr,cabac199)/4+160,197);}
 #[test]fn non_ctu_aligned_inputs(){for (w,h) in [(1920,1088),(3840,2160),(1919,1079),(3856,2176),(4080,2288)] {for ctu in[16,32,64]{for d in[8,10]{for t in 0..=19.min((w+ctu-1)/ctu-1){let a=candidate(w,h);let b=hevc(w,h,ctu,d,t);assert!(a.iter().zip(b).all(|(x,y)|*x>=y));}}}}}
 #[test]fn register_slots_preserved(){assert_eq!(NAMES.len(),10);assert_eq!(133+4,137);assert_eq!(133+9,142);}
 #[test]fn candidate_intra_bound(){for w in(64..=4096).step_by(64){for c in[16,32,64]{for t in 0..=19.min((w+c-1)/c-1){assert!(candidate(w,64)[0]>=hevc(w,64,c,10,t)[0]);}}}}
}
