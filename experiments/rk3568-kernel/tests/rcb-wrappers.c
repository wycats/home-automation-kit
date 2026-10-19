/* SPDX-License-Identifier: GPL-2.0-only */
/* Host FFI boundaries only; the size functions and tables are extracted upstream C. */
size_t candidate_size(unsigned slot,unsigned w,unsigned h) { return rkvdec_rcb_size(&vdpu346_rcb_sizes[slot],w,h); }
size_t original_381_size(unsigned slot,unsigned w,unsigned h) { return original_size(&original_rcb_sizes[slot],w,h); }
size_t unchanged_381_size(unsigned slot,unsigned w,unsigned h) { return rkvdec_rcb_size(&vdpu381_rcb_sizes[slot],w,h); }
size_t unchanged_383_size(unsigned slot,unsigned w,unsigned h) { return rkvdec_rcb_size(&vdpu383_rcb_sizes[slot],w,h); }
size_t original_383_size(unsigned slot,unsigned w,unsigned h) { return original_size(&original_383_rcb_sizes[slot],w,h); }
unsigned original_383_slots(void) { return sizeof(original_383_rcb_sizes)/sizeof(original_383_rcb_sizes[0]); }
unsigned candidate_slots(void) { return sizeof(vdpu346_rcb_sizes)/sizeof(vdpu346_rcb_sizes[0]); }
unsigned candidate_axis(unsigned slot) { return vdpu346_rcb_sizes[slot].axis; }
unsigned candidate_alignment(unsigned slot) { return vdpu346_rcb_sizes[slot].dimension_alignment; }
