/* SPDX-License-Identifier: GPL-2.0 */
/* Actual backend run function with request, MMIO and assembler boundary mocks. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <errno.h>
typedef uint32_t u32;
struct rkvdec_run { int unused; };
struct v4l2_ctrl_hevc_sps { int num_long_term_ref_pics_sps, num_short_term_ref_pic_sets; };
struct rkvdec_hevc_run { struct rkvdec_run base; struct v4l2_ctrl_hevc_sps *sps; };
struct rkvdec_dev { void *dev; unsigned char *regs, *link; };
struct rkvdec_hevc_priv_tbl { int scaling_list,rps; };
struct rkvdec_hevc_ctx {
 struct { void *cpu; } priv_tbl;
 struct { struct { u32 reg032_timeout_threshold,reg013_core_timeout_threshold; } common; } regs;
 int scaling_matrix_cache,st_cache;
};
struct rkvdec_ctx { struct rkvdec_dev *dev; struct rkvdec_hevc_ctx *priv; int has_sps_lt_rps,has_sps_st_rps; };
static int pre,post,config,writes,watchdog,assembler_result,tests,failures;
static struct v4l2_ctrl_hevc_sps sps;
static void rkvdec_hevc_run_preamble(struct rkvdec_ctx *c, struct rkvdec_hevc_run *r) { (void)c; pre++;r->sps=&sps; }
static void rkvdec_run_postamble(struct rkvdec_ctx *c, struct rkvdec_run *r) { (void)c;(void)r; post++; }
static void rkvdec_hevc_assemble_hw_scaling_list(struct rkvdec_ctx *c,struct rkvdec_hevc_run *r,int *t,int *cache) { (void)c;(void)r;(void)t;(void)cache; }
static void assemble_hw_pps(struct rkvdec_ctx *c,struct rkvdec_hevc_run *r) { (void)c;(void)r; }
static void config_registers(struct rkvdec_ctx *c,struct rkvdec_hevc_run *r) { (void)c;(void)r; config++; }
#if FIXED
static int rkvdec_hevc_assemble_hw_rps(struct rkvdec_hevc_run *r,int *t) { (void)r;(void)t; return assembler_result; }
#else
static void rkvdec_hevc_assemble_hw_rps(struct rkvdec_hevc_run *r,int *t,int *cache) { (void)r;(void)t;(void)cache; }
#endif
static void rkvdec_schedule_watchdog(struct rkvdec_dev *d,u32 t) { (void)d;(void)t; watchdog++; }
static void writel(u32 val,void *addr) { (void)val;(void)addr; writes++; }
#define dev_err_ratelimited(...) ((void)0)
#define dev_warn_ratelimited(...) ((void)0)
#define VDPU381_DEC_E_BIT 1
#define VDPU381_REG_DEC_E 0
#define VDPU383_LINK_TIMEOUT_THRESHOLD 0
#define VDPU383_IP_CRU_MODE 1
#define VDPU383_LINK_IP_ENABLE 4
#define VDPU383_DEC_E_BIT 1
#define VDPU383_LINK_DEC_ENABLE 8
#include "backend.inc"
static void testcase(const char *name,int missing,int assembly,int expect) {
 unsigned char regs[64]={0};
 struct rkvdec_dev dev={.regs=regs,.link=regs};
 struct rkvdec_hevc_priv_tbl tbl={0};
 struct rkvdec_hevc_ctx priv={.priv_tbl={.cpu=&tbl}};
 struct rkvdec_ctx ctx={.dev=&dev,.priv=&priv,.has_sps_lt_rps=1,.has_sps_st_rps=!missing};
 sps.num_short_term_ref_pic_sets=1; sps.num_long_term_ref_pics_sps=0;
 pre=post=config=writes=watchdog=0;assembler_result=assembly;
 int ret=rkvdec_hevc_run(&ctx);
 int ok=ret==expect && pre==1 && post==1;
 if(expect) ok &= config==0 && writes==0 && watchdog==0;
 else ok &= config==1 && writes==(BACKEND==381 ? 1:3) && watchdog==1;
 tests++; failures+=!ok;
 printf("%s %s ret=%d pre=%d post=%d config=%d writes=%d watchdog=%d\n",ok ? "PASS":"FAIL",name,ret,pre,post,config,writes,watchdog);
}
int main(void) {
 testcase("successful_request_completed_and_started_once",0,0,0);
 testcase("missing_controls_complete_without_hardware_start",1,0,-EINVAL);
 testcase("allocation_error_completes_without_hardware_start",0,-ENOMEM,-ENOMEM);
 testcase("invalid_rps_completes_without_hardware_start",0,-EINVAL,-EINVAL);
 printf("RESULT tests=%d failures=%d\n",tests,failures);
 return failures ? 1:0;
}
