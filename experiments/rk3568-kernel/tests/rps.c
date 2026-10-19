/* SPDX-License-Identifier: GPL-2.0 */
/* Host-only regression driver. Kernel routines are extracted into subject.inc. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
typedef uint8_t u8; typedef uint16_t u16; typedef uint32_t u32; typedef uint64_t u64;
typedef int16_t s16; typedef int32_t s32;
typedef u8 __u8; typedef u16 __u16; typedef u32 __u32;
#define __packed
#define ALIGN(x,a) (((x)+(a)-1)&~((a)-1))
#define GENMASK_ULL(h,l) ((UINT64_MAX << (l)) & (UINT64_MAX >> (63-(h))))
#define GFP_KERNEL 0
#define V4L2_HEVC_EXT_SPS_ST_RPS_FLAG_INTER_REF_PIC_SET_PRED 1
#define V4L2_HEVC_EXT_SPS_LT_RPS_FLAG_USED_LT 1
static int fail_alloc, allocations, frees;
static void *kcalloc(size_t n,size_t size,int flags) { (void)flags; allocations++; return fail_alloc ? NULL : calloc(n,size); }
static void kfree(void *p) { frees++; free(p); }
/* Only the two SPS fields read by these extracted routines are modeled. */
struct v4l2_ctrl_hevc_sps { u8 num_short_term_ref_pic_sets, num_long_term_ref_pics_sps; };
#include "subject.inc"

static struct v4l2_ctrl_hevc_sps sps;
static struct v4l2_ctrl_hevc_ext_sps_st_rps st[64], cache;
static struct v4l2_ctrl_hevc_ext_sps_lt_rps lt[32];
static struct rkvdec_hevc_run run;
static struct { u64 pre[4]; struct rkvdec_rps table; u64 post[4]; } guarded;
static int tests, failures;
static void reset(void) {
 memset(&sps,0,sizeof(sps)); memset(st,0,sizeof(st)); memset(lt,0,sizeof(lt));
 memset(&cache,0,sizeof(cache)); memset(&guarded,0,sizeof(guarded));
 for(int i=0;i<4;i++) guarded.pre[i]=guarded.post[i]=UINT64_C(0x12345678abcdef90);
 run.sps=&sps; run.ext_sps_st_rps=st; run.ext_sps_lt_rps=lt;
 fail_alloc=allocations=frees=0;
}
static int call(void) {
#if FIXED
 return rkvdec_hevc_assemble_hw_rps(&run,&guarded.table);
#else
 rkvdec_hevc_assemble_hw_rps(&run,&guarded.table,&cache); return 0;
#endif
}
static u32 bits(struct rkvdec_bw_field f) {
 u32 value=0;
 for(int i=0;i<f.len;i++) value|=((guarded.table.info[(f.offset+i)/32]>>((f.offset+i)%32))&1)<<i;
 return value;
}
static void check(const char *name,int ok) {
 int canary=1; for(int i=0;i<4;i++) canary &= guarded.pre[i]==UINT64_C(0x12345678abcdef90) && guarded.post[i]==UINT64_C(0x12345678abcdef90);
 ok &= canary; tests++; failures+=!ok;
 printf("%s %s\n",ok ? "PASS":"FAIL",name);
}
static void direct(int index,int count) { st[index].num_negative_pics=(u8)count; st[index].used_by_curr_pic=0xffff; }
static void rejected(const char *name,int expected) {
 struct rkvdec_rps before; memset(&guarded.table,0xa5,sizeof(guarded.table)); before=guarded.table;
 int r=call(); check(name,r==expected && memcmp(&before,&guarded.table,sizeof(before))==0);
}
int main(void) {
 reset(); sps.num_short_term_ref_pic_sets=2; direct(0,1); direct(1,1);
 call(); st[1].delta_poc_s0_minus1[0]=5; call();
 check("changed_second_set_rebuilt",(s16)bits(RPS_ST_REF_SET_DELTA_POC(1,0))==-6);

 reset(); sps.num_short_term_ref_pic_sets=2; direct(1,1); call();
 check("zero_first_set_does_not_hide_second",bits(RPS_ST_REF_SET_NUM_NEGATIVE(1))==1);

 reset(); sps.num_short_term_ref_pic_sets=1; direct(0,1); fail_alloc=1;
 rejected("allocation_failure_propagates_without_table_write",-ENOMEM);

 reset(); sps.num_short_term_ref_pic_sets=1; st[0].flags=1;
 rejected("invalid_prediction_index_propagates_without_table_write",-EINVAL);

 reset(); sps.num_short_term_ref_pic_sets=2; direct(0,15);
 st[1].flags=1; st[1].delta_rps_sign=1; st[1].use_delta_flag=st[1].used_by_curr_pic=0xffff;
 rejected("final_prediction_cannot_truncate_sixteen_to_zero",-EINVAL);

 reset(); sps.num_short_term_ref_pic_sets=1; run.ext_sps_st_rps=NULL;
 rejected("missing_short_term_control_rejected",-EINVAL);
 reset(); sps.num_long_term_ref_pics_sps=1; run.ext_sps_lt_rps=NULL;
 rejected("missing_long_term_control_rejected",-EINVAL);

 reset(); sps.num_short_term_ref_pic_sets=2; direct(0,1);
 st[0].delta_poc_s0_minus1[0]=1; st[1].flags=1; st[1].use_delta_flag=st[1].used_by_curr_pic=3;
 sps.num_long_term_ref_pics_sps=1; lt[0].lt_ref_pic_poc_lsb_sps=321; lt[0].flags=1;
 int r=call();
 check("valid_prediction_and_long_term_bits",r==0 && bits(RPS_ST_REF_SET_NUM_NEGATIVE(1))==1 && bits(RPS_ST_REF_SET_NUM_POSITIVE(1))==1 && (s16)bits(RPS_ST_REF_SET_DELTA_POC(1,0))==-1 && bits(RPS_ST_REF_SET_DELTA_POC(1,1))==1 && bits(RPS_LT_REF_PIC_POC_LSB(0))==321 && bits(RPS_LT_REF_USED_BY_CURR_PIC(0))==1);
 sps.num_short_term_ref_pic_sets=0; sps.num_long_term_ref_pics_sps=0;
 run.ext_sps_st_rps=NULL; run.ext_sps_lt_rps=NULL; int old_alloc=allocations; r=call();
 struct rkvdec_rps zero={0}; check("zero_sets_clear_stale_table_without_allocation",r==0 && allocations==old_alloc && memcmp(&zero,&guarded.table,sizeof(zero))==0);

 reset(); sps.num_short_term_ref_pic_sets=64; sps.num_long_term_ref_pics_sps=32;
 for(int i=0;i<64;i++) direct(i,15);
 lt[31].lt_ref_pic_poc_lsb_sps=1234; lt[31].flags=1; r=call();
 check("maximum_valid_table_and_canaries",r==0 && bits(RPS_ST_REF_SET_NUM_NEGATIVE(63))==15 && (s16)bits(RPS_ST_REF_SET_DELTA_POC(63,14))==-15 && bits(RPS_LT_REF_PIC_POC_LSB(31))==1234 && allocations==frees);

#if FIXED
 /* Do not deliberately execute unsafe out-of-bounds inputs against the baseline. */
 reset(); sps.num_short_term_ref_pic_sets=65; rejected("too_many_short_term_sets",-EINVAL);
 reset(); sps.num_long_term_ref_pics_sps=33; rejected("too_many_long_term_sets",-EINVAL);
 reset(); sps.num_short_term_ref_pic_sets=1; direct(0,16); rejected("direct_set_overflow",-EINVAL);
 /* Exercise changes in all later sets and counts with an independent bit reader. */
 int sweep=1;
 for(int count=1;count<=64;count++) {
  reset(); sps.num_short_term_ref_pic_sets=(u8)count;
  for(int i=0;i<count;i++) direct(i,1);
  for(int delta=0;delta<16;delta++) {
   st[count-1].delta_poc_s0_minus1[0]=(u16)delta;
   sweep &= call()==0 && (s16)bits(RPS_ST_REF_SET_DELTA_POC(count-1,0))==-(delta+1);
  }
  sweep &= allocations==frees;
 }
 check("1024_later_set_rebuilds",sweep);
#endif
 printf("RESULT tests=%d failures=%d\n",tests,failures);
 return failures ? 1:0;
}
