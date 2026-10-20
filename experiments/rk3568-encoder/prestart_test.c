/* SPDX-License-Identifier: GPL-2.0 */
/* Host fault injection around extracted, unmodified candidate C functions.
 * Resource APIs and hardware tail are mocked; no device is opened.
 */
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "hantro_h264_enc_validate.h"

typedef uint64_t u64;
#include "hantro_h264_enc_ref.h"
typedef uint32_t u32;
typedef int32_t s32;
typedef uint8_t u8;
#define HANTRO_MODE_H264_ENC 1
#define HANTRO_MODE_JPEG_ENC 2
#define HANTRO_H264_ENC_CABAC_TABLE_COUNT 3
#define V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE 9
#define VB2_BUF_STATE_ERROR 7
#define WARN_ON(x) (!!(x))
#define MB_DIM 16U
#define MB_WIDTH(x) ((x) / MB_DIM)
#define MB_HEIGHT(x) ((x) / MB_DIM)
#define round_up(x, y) (((x) + (y) - 1) & ~((y) - 1))

enum vb2_buffer_state { VB2_BUF_STATE_DONE };
struct work_struct { void (*fn)(struct work_struct *); };
struct delayed_work { struct work_struct work; };
#define INIT_WORK(w, f) ((w)->fn = (f))
#define INIT_DELAYED_WORK(w, f) INIT_WORK(&(w)->work, f)
#define GFP_KERNEL 0
#define spin_lock_irqsave(l, f) do { (void)(l); (f) = 0; } while (0)
#define spin_unlock_irqrestore(l, f) do { (void)(l); (void)(f); } while (0)
static void hantro_h264_timeout(struct work_struct *w) { (void)w; }
static void hantro_h264_complete(struct work_struct *w) { (void)w; }
static int allocation_failure;
static void *kzalloc(size_t n, int flag) { (void)flag; return allocation_failure ? NULL : calloc(1, n); }
static void kfree(void *p) { free(p); }
struct hantro_h264_job;
struct hantro_ctx;
struct request_object { void *req; };
struct vb2_buffer { struct request_object req_obj; u64 timestamp; };
struct vb2_v4l2_buffer { struct vb2_buffer vb2_buf; };
struct vb2_queue { int unused; };
struct hantro_aux_buf { void *cpu; unsigned long size; uint64_t dma; };
struct hantro_enc_buf { struct vb2_v4l2_buffer base; struct hantro_aux_buf rec_buf; struct hantro_h264_ref h264_ref; };
struct hantro_h264_enc_ctrls {
    const struct v4l2_ctrl_h264_encode_params *encode;
    const struct v4l2_ctrl_h264_encode_rc *rc;
};
struct hantro_h264_enc_hw_ctx {
    struct hantro_h264_enc_ctrls ctrls;
    struct hantro_aux_buf cabac_table[HANTRO_H264_ENC_CABAC_TABLE_COUNT];
};
struct v4l2_pix_format_mplane { unsigned int width, height; };
struct variant { int num_clocks; };
struct hantro_dev { void *dev, *clocks; struct variant *variant;
    int irqlock; struct hantro_h264_job *h264_job; bool h264_irq_mode, h264_mmio_live;
};
struct codec_ops { int (*run)(struct hantro_ctx *); void (*reset)(struct hantro_ctx *); };
struct format { int codec_mode; };
struct hantro_ctx {
    struct hantro_dev *dev;
    struct codec_ops *codec_ops;
    struct format *vpu_dst_fmt;
    bool is_encoder, h264_abort;
    struct hantro_h264_ref_domain h264_refs;
    struct hantro_h264_job *h264_job;
    struct hantro_h264_enc_hw_ctx h264_enc;
    struct v4l2_pix_format_mplane src_fmt;
    struct { void *m2m_ctx; } fh;
    int ctrl_handler;
};

static struct hantro_enc_buf source, destination, reference;
static struct vb2_queue capture_queue;
static struct v4l2_ctrl_h264_encode_params params;
static struct v4l2_ctrl_h264_encode_rc rate;
static int pm_result, clock_result, setup_result, reference_kind;
static int live_pm, live_clocks, request_completions, finishes, starts, watches;
static int control_reads, missing_control;
static char trace[64];

static void event(char e)
{
    size_t n = strlen(trace);
    assert(n + 1 < sizeof(trace)); trace[n] = e; trace[n + 1] = 0;
}
static struct vb2_v4l2_buffer *hantro_get_src_buf(struct hantro_ctx *ctx)
{ (void)ctx; return &source.base; }
static struct vb2_v4l2_buffer *hantro_get_dst_buf(struct hantro_ctx *ctx)
{ (void)ctx; return &destination.base; }
static struct hantro_enc_buf *hantro_get_enc_buf(struct vb2_v4l2_buffer *b)
{ return (struct hantro_enc_buf *)b; }
static struct vb2_v4l2_buffer *to_vb2_v4l2_buffer(struct vb2_buffer *b)
{ return (struct vb2_v4l2_buffer *)b; }
static int pm_runtime_resume_and_get(void *dev)
{ (void)dev; event('P'); if (pm_result >= 0) live_pm++; return pm_result; }
static void pm_runtime_put_autosuspend(void *dev)
{ (void)dev; event('p'); assert(live_pm == 1); live_pm--; }
static int clk_bulk_enable(int n, void *clocks)
{ (void)n; (void)clocks; event('C'); if (!clock_result) live_clocks++; return clock_result; }
static void clk_bulk_disable(int n, void *clocks)
{ (void)n; (void)clocks; event('c'); assert(live_clocks == 1); live_clocks--; }
static void v4l2_m2m_buf_copy_metadata(struct vb2_v4l2_buffer *a, struct vb2_v4l2_buffer *b)
{ assert(a == &source.base && b == &destination.base); event('M'); }
static int v4l2_ctrl_request_setup(void *req, int *handler)
{ assert(req == &source && handler); event('S'); return setup_result; }
static void v4l2_ctrl_request_complete(void *req, int *handler)
{ assert(req == &source && handler); event('R'); assert(++request_completions == 1); }
static void hantro_job_finish_no_pm(struct hantro_dev *vpu, struct hantro_ctx *ctx, int state)
{
    assert(vpu == ctx->dev && state == VB2_BUF_STATE_ERROR);
    assert(request_completions == 1 && live_pm == 0 && live_clocks == 0);
    event('F'); assert(++finishes == 1);
}
static void *hantro_get_ctrl(struct hantro_ctx *ctx, u32 id)
{
    (void)ctx; control_reads++;
    if (id == V4L2_CID_STATELESS_H264_ENCODE_PARAMS)
        return missing_control == 1 ? NULL : &params;
    assert(id == V4L2_CID_STATELESS_H264_ENCODE_RC);
    return missing_control == 2 ? NULL : &rate;
}
static struct vb2_queue *v4l2_m2m_get_vq(void *ctx, unsigned int type)
{ (void)ctx; assert(type == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE); return &capture_queue; }
static unsigned int vb2_get_max_num_buffers(struct vb2_queue *q)
{ assert(q == &capture_queue); return 1; }
static struct vb2_buffer *vb2_get_buffer(struct vb2_queue *q, unsigned int i)
{
    assert(q == &capture_queue && i == 0);
    if (reference_kind == 0) return NULL;
    return reference_kind == 1 ? &destination.base.vb2_buf : &reference.base.vb2_buf;
}
static void synthetic_hardware_commit(struct hantro_ctx *ctx)
{ (void)ctx; assert(live_pm == 1 && live_clocks == 1); event('W'); watches++; event('E'); starts++; }

/* Generated from the exact patched source, preserving its license headers. */
#include "prestart_subject.inc"

static int other_codec_run(struct hantro_ctx *ctx) { (void)ctx; event('O'); return 0; }

static void reset(struct hantro_ctx *ctx)
{
    kfree(ctx->h264_job); ctx->h264_job = NULL; ctx->dev->h264_job = NULL;
    ctx->h264_abort = ctx->dev->h264_mmio_live = false; allocation_failure = 0;
    memset(trace, 0, sizeof(trace));
    pm_result = clock_result = setup_result = reference_kind = 0;
    live_pm = live_clocks = request_completions = finishes = starts = watches = 0;
    control_reads = missing_control = 0;
    memset(&source, 0, sizeof(source)); memset(&destination, 0, sizeof(destination));
    memset(&reference, 0, sizeof(reference));
    source.base.vb2_buf.req_obj.req = &source;
    destination.rec_buf.cpu = &destination; destination.rec_buf.size = 640 * 480 * 3 / 2;
    reference.rec_buf.cpu = &reference; reference.rec_buf.size = destination.rec_buf.size;
    memset(&params, 0, sizeof(params)); params.slice_type = V4L2_H264_SLICE_TYPE_I;
    memset(&rate, 0, sizeof(rate)); rate.qp = 26; rate.qp_max = 51;
    ctx->src_fmt.width = 640; ctx->src_fmt.height = 480;
    ctx->h264_refs = (struct hantro_h264_ref_domain){ .epoch = 1 };
    assert(hantro_h264_ref_publish(&reference.h264_ref, &ctx->h264_refs, 1, params.reference_ts));
    assert(hantro_h264_ref_publish(&destination.h264_ref, &ctx->h264_refs, 1, params.reference_ts));
    memset(&ctx->h264_enc, 0, sizeof(ctx->h264_enc));
    ctx->is_encoder = true; ctx->vpu_dst_fmt->codec_mode = HANTRO_MODE_H264_ENC;
    ctx->codec_ops->run = rk3399_vpu_h264_enc_run;
}

static void expect_error(struct hantro_ctx *ctx, const char *expected)
{
    device_run(ctx);
    assert(strcmp(trace, expected) == 0);
    assert(request_completions == 1 && finishes == 1);
    assert(!live_pm && !live_clocks && !starts && !watches);
    assert(!destination.h264_ref.valid && reference.h264_ref.valid && ctx->h264_refs.epoch == 1);
}

int main(void)
{
    struct variant variant = {1};
    struct hantro_dev dev = {0};
    struct codec_ops ops = {0};
    struct format fmt = {0};
    struct hantro_ctx ctx = {0};
    unsigned int i;
    dev.variant = &variant; ctx.dev = &dev; ctx.codec_ops = &ops; ctx.vpu_dst_fmt = &fmt;

    reset(&ctx); allocation_failure = 1; expect_error(&ctx, "RF");
    reset(&ctx); ctx.h264_abort = true; expect_error(&ctx, "RF");
    reset(&ctx); pm_result = -EIO; expect_error(&ctx, "PRF");
    reset(&ctx); clock_result = -EIO; expect_error(&ctx, "PCRpF");
    reset(&ctx); setup_result = -EINVAL; expect_error(&ctx, "PCMSRcpF"); assert(!control_reads);
    for (i = 1; i <= 2; i++) {
        reset(&ctx); missing_control = (int)i; expect_error(&ctx, "PCMSRcpF");
    }
    reset(&ctx); rate.qp = 52; expect_error(&ctx, "PCMSRcpF");
    reset(&ctx); ctx.src_fmt.width = 3840; expect_error(&ctx, "PCMSRcpF");
    reset(&ctx); destination.rec_buf.size--; expect_error(&ctx, "PCMSRcpF");
    reset(&ctx); destination.rec_buf.cpu = NULL; expect_error(&ctx, "PCMSRcpF");
    reset(&ctx); params.flags = V4L2_H264_ENCODE_FLAG_ENTROPY_CODING_MODE;
    expect_error(&ctx, "PCMSRcpF");
    reset(&ctx); params.cabac_init_idc = 3; expect_error(&ctx, "PCMSRcpF");
    for (i = 0; i < 4; i++) {
        reset(&ctx); params.slice_type = V4L2_H264_SLICE_TYPE_P;
        reference_kind = i < 2 ? (int)i : 2;
        if (i == 2) reference.rec_buf.size--;
        if (i == 3) reference.rec_buf.cpu = NULL;
        expect_error(&ctx, "PCMSRcpF");
    }
    /* Successful starts transfer resources; they must not synchronously retire. */
    for (i = 0; i < 3; i++) {
        reset(&ctx); pm_result = 1; /* Positive PM success is valid. */
        if (i == 1) { params.slice_type = V4L2_H264_SLICE_TYPE_P; reference_kind = 2; }
        if (i == 2) {
            params.flags = V4L2_H264_ENCODE_FLAG_ENTROPY_CODING_MODE;
            ctx.h264_enc.cabac_table[0].cpu = &ctx;
        }
        device_run(&ctx);
        assert(strcmp(trace, "PCMSWE") == 0);
        assert(live_pm == 1 && live_clocks == 1 && starts == 1 && watches == 1);
        assert(!request_completions && !finishes);
    }
    /* Execute the actual dispatcher to verify JPEG and decoder bypass the new path. */
    for (i = 0; i < 2; i++) {
        reset(&ctx); ctx.codec_ops->run = other_codec_run;
        if (!i) ctx.vpu_dst_fmt->codec_mode = HANTRO_MODE_JPEG_ENC;
        else ctx.is_encoder = false;
        device_run(&ctx);
        assert(strcmp(trace, "PCMO") == 0);
        assert(live_pm == 1 && live_clocks == 1 && !request_completions && !finishes);
    }
    puts("17 actual-path rejection cases, 3 successful handoffs and 2 non-H264 dispatch checks passed; hardware tail mocked, no device access.");
    return 0;
}
