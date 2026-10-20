/* SPDX-License-Identifier: GPL-2.0 */
/* Host doubles for exact candidate buffer, lookup and allocation ioctl paths. */
#include <assert.h>
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "hantro_h264_enc_ref.h"
#define HANTRO_MODE_H264_ENC 1
#define HANTRO_MODE_JPEG_ENC 2
#define V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE 9
#define V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE 10
#define V4L2_MEMORY_MMAP 1
#define V4L2_MEMORY_DMABUF 4
#define V4L2_H264_ENCODE_FLAG_ENTROPY_CODING_MODE 1
#define V4L2_H264_SLICE_TYPE_P 0
#define V4L2_H264_SLICE_TYPE_I 2
#define V4L2_FIELD_NONE 0
#define V4L2_TYPE_IS_OUTPUT(t) ((t) == V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE)
#define V4L2_TYPE_IS_CAPTURE(t) ((t) == V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE)
#define GFP_KERNEL 0
#define MB_DIM 16U
#define MB_WIDTH(x) ((x) / MB_DIM)
#define MB_HEIGHT(x) ((x) / MB_DIM)
#define round_up(x, y) (((x) + (y) - 1) & ~((y) - 1))
static unsigned int held;
#define spin_lock_irqsave(l, f) do { (void)(l); (f) = 0; assert(!held); held++; } while (0)
#define spin_unlock_irqrestore(l, f) do { (void)(l); (void)(f); assert(held == 1); held--; } while (0)
struct hantro_ctx;
struct vb2_queue { unsigned int type, max_num_buffers; bool streaming; struct hantro_ctx *ctx; };
struct vb2_buffer { struct vb2_queue *vb2_queue; unsigned int num_planes; size_t payload; uint64_t timestamp; };
struct vb2_v4l2_buffer { struct vb2_buffer vb2_buf; unsigned int field, sequence; };
struct hantro_aux_buf { void *cpu; size_t size; uint64_t dma; };
struct hantro_enc_buf { struct vb2_v4l2_buffer base; struct hantro_aux_buf rec_buf; struct hantro_h264_ref h264_ref; };
struct v4l2_ctrl_h264_encode_params { unsigned int flags, cabac_init_idc, slice_type; uint64_t reference_ts; };
struct hantro_h264_enc_hw_ctx {
    struct { const struct v4l2_ctrl_h264_encode_params *encode; } ctrls;
    struct hantro_aux_buf cabac_table[3];
};
struct v4l2_pix_format_mplane { unsigned int width, height; struct { unsigned int sizeimage; } plane_fmt[1]; };
struct format { int codec_mode; };
struct hantro_dev { int irqlock; void *dev; };
struct hantro_ctx {
    struct hantro_dev *dev; bool is_encoder;
    struct format *vpu_dst_fmt; struct v4l2_pix_format_mplane src_fmt, dst_fmt;
    struct { void *m2m_ctx; } fh;
    struct hantro_h264_enc_hw_ctx h264_enc;
    struct hantro_h264_ref_domain h264_refs;
    unsigned int sequence_cap;
};
struct file { struct hantro_ctx *ctx; };
struct v4l2_requestbuffers { unsigned int type, memory, count; };
struct v4l2_create_buffers { struct { unsigned int type; } format; unsigned int memory, count; };
struct v4l2_remove_buffers { unsigned int type, count; };
static struct vb2_queue cap, out;
static struct hantro_enc_buf buffers[3];
static struct vb2_buffer *slots[3];
static int allocation_fail, allocations, releases, plane_error, queues, last_buffers, delegates;
static bool last_buffer;
static struct hantro_ctx *file_to_ctx(struct file *f) { return f->ctx; }
static struct hantro_ctx *vb2_get_drv_priv(struct vb2_queue *q) { return q->ctx; }
static struct vb2_v4l2_buffer *to_vb2_v4l2_buffer(struct vb2_buffer *b) { return (void *)b; }
static struct hantro_enc_buf *hantro_get_enc_buf(struct vb2_v4l2_buffer *b) { return (void *)b; }
static struct vb2_queue *v4l2_m2m_get_src_vq(void *m) { (void)m; return &out; }
static struct vb2_queue *v4l2_m2m_get_dst_vq(void *m) { (void)m; return &cap; }
static struct vb2_queue *v4l2_m2m_get_vq(void *m, unsigned int type) { (void)m; assert(type == cap.type); return &cap; }
static bool vb2_is_streaming(struct vb2_queue *q) { return q->streaming; }
static struct vb2_buffer *vb2_get_buffer(struct vb2_queue *q, unsigned int i) { assert(q == &cap && held && i < 3); return slots[i]; }
static void vb2_set_plane_payload(struct vb2_buffer *b, unsigned int plane, size_t size) { assert(plane == 0); b->payload = size; }
static int hantro_buf_plane_check(struct vb2_buffer *b, struct v4l2_pix_format_mplane *f) { (void)b; (void)f; return plane_error; }
static void *dma_alloc_coherent(void *d, size_t n, uint64_t *dma, int flag) {
    (void)d; (void)flag; assert(!held); if (allocation_fail) return NULL;
    void *p = malloc(n); assert(p); memset(p, 0xa5, n); *dma = 1234; allocations++; return p;
}
static void dma_free_coherent(void *d, size_t n, void *p, uint64_t dma) { (void)d; (void)n; assert(!held && p && dma == 1234); releases++; free(p); }
static bool v4l2_m2m_dst_buf_is_last(void *m) { (void)m; return last_buffer; }
static void v4l2_m2m_last_buffer_done(void *m, struct vb2_v4l2_buffer *b) { (void)m; (void)b; last_buffers++; }
static void v4l2_m2m_buf_queue(void *m, struct vb2_v4l2_buffer *b) { (void)m; (void)b; queues++; }
static const int hantro_eos_event = 0;
static void v4l2_event_queue_fh(void *fh, const int *event) { (void)fh; assert(event == &hantro_eos_event); }
/* Forwarding doubles reflect only the relevant pinned vb2 contract. */
static int v4l2_m2m_ioctl_reqbufs(struct file *f, void *p, struct v4l2_requestbuffers *r) {
    (void)f; (void)p; delegates++; return r->type == cap.type && cap.streaming ? -EBUSY : 0;
}
static int v4l2_m2m_ioctl_create_bufs(struct file *f, void *p, struct v4l2_create_buffers *r) { (void)f; (void)p; (void)r; delegates++; return 0; }
static int v4l2_m2m_ioctl_remove_bufs(struct file *f, void *p, struct v4l2_remove_buffers *r) { (void)f; (void)p; (void)r; delegates++; return 0; }
#include "reference_subject.inc"

static void publish(struct hantro_ctx *ctx, unsigned int i, uint64_t ts) {
    assert(hantro_h264_ref_publish(&buffers[i].h264_ref, &ctx->h264_refs, ctx->h264_refs.epoch, ts));
}
static void expect_ref(struct hantro_ctx *ctx, int result, int index) {
    struct hantro_aux_buf *ref = (void *)1;
    assert(rk3399_vpu_h264_enc_prepare_buffers(ctx, &buffers[2].base, &ref) == result);
    assert(ref == (index < 0 ? NULL : &buffers[index].rec_buf));
}
int main(void) {
    struct hantro_dev dev = {0}; struct format fmt = {HANTRO_MODE_H264_ENC};
    struct hantro_ctx ctx = {.dev=&dev, .is_encoder=true, .vpu_dst_fmt=&fmt};
    struct v4l2_ctrl_h264_encode_params params = {.slice_type=V4L2_H264_SLICE_TYPE_P, .reference_ts=100};
    ctx.h264_enc.ctrls.encode = &params; ctx.src_fmt.width = 64; ctx.src_fmt.height = 64;
    cap = (struct vb2_queue){.type=V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, .ctx=&ctx, .max_num_buffers=3};
    out = (struct vb2_queue){.type=V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, .ctx=&ctx};
    assert(hantro_h264_ref_advance(&ctx.h264_refs));
    for (unsigned int i=0; i<3; i++) {
        slots[i] = &buffers[i].base.vb2_buf; slots[i]->vb2_queue=&cap; slots[i]->num_planes=1;
        assert(!hantro_buf_init(slots[i])); assert(!buffers[i].h264_ref.valid);
    }
    /* A copied generic timestamp is not a successful reconstruction. */
    slots[0]->timestamp = 100; expect_ref(&ctx, -EINVAL, -1);
    publish(&ctx, 0, 100); expect_ref(&ctx, 0, 0);
    publish(&ctx, 1, 100); expect_ref(&ctx, -EINVAL, -1);
    hantro_h264_invalidate_buffer(slots[1]); expect_ref(&ctx, 0, 0);
    publish(&ctx, 2, 100); expect_ref(&ctx, -EINVAL, -1);
    hantro_h264_invalidate_buffer(slots[2]); expect_ref(&ctx, 0, 0);
    buffers[0].rec_buf.size--; expect_ref(&ctx, -EINVAL, -1); buffers[0].rec_buf.size++;
    void *saved = buffers[0].rec_buf.cpu; buffers[0].rec_buf.cpu=NULL;
    expect_ref(&ctx, -EINVAL, -1); buffers[0].rec_buf.cpu=saved;
    slots[1]=NULL; expect_ref(&ctx, 0, 0); slots[1]=&buffers[1].base.vb2_buf;
    /* Requeue withdraws future eligibility, not bytes already held by job. */
    struct hantro_aux_buf *held_ref=NULL;
    assert(!rk3399_vpu_h264_enc_prepare_buffers(&ctx, &buffers[2].base, &held_ref));
    int freed_before=releases;
    assert(!hantro_buf_prepare(slots[0])); hantro_buf_queue(slots[0]);
    assert(!buffers[0].h264_ref.valid && releases==freed_before && held_ref->cpu==saved);
    for (size_t n=0; n<held_ref->size; n++) assert(((unsigned char *)held_ref->cpu)[n]==0xa5);
    expect_ref(&ctx, -EINVAL, -1);
    /* Prepared earlier: buf_queue must independently withdraw eligibility. */
    publish(&ctx, 0, 100); hantro_buf_queue(slots[0]); expect_ref(&ctx, -EINVAL, -1);
    publish(&ctx, 0, 100); plane_error=-EINVAL;
    assert(hantro_buf_prepare(slots[0])==-EINVAL); expect_ref(&ctx, -EINVAL, -1); plane_error=0;
    publish(&ctx, 0, 100); cap.streaming=true; last_buffer=true;
    hantro_buf_queue(slots[0]); assert(last_buffers==1 && !buffers[0].h264_ref.valid);
    cap.streaming=false; last_buffer=false;
    publish(&ctx, 0, 100); hantro_buf_cleanup(slots[0]);
    assert(!buffers[0].h264_ref.valid && !buffers[0].rec_buf.cpu && !buffers[0].rec_buf.size);
    allocation_fail=1; assert(hantro_buf_init(slots[0])==-ENOMEM); assert(!buffers[0].h264_ref.valid);
    allocation_fail=0; assert(!hantro_buf_init(slots[0])); expect_ref(&ctx, -EINVAL, -1);
    publish(&ctx, 0, 101); expect_ref(&ctx, -EINVAL, -1); params.reference_ts=101; expect_ref(&ctx, 0, 0);
    assert(hantro_h264_ref_advance(&ctx.h264_refs)); expect_ref(&ctx, -EINVAL, -1);
    assert(!hantro_h264_ref_publish(&buffers[0].h264_ref, &ctx.h264_refs, 1, 101));
    ctx.h264_refs.epoch=UINT64_MAX; publish(&ctx, 0, 101);
    assert(!hantro_h264_ref_advance(&ctx.h264_refs)); expect_ref(&ctx, -EINVAL, -1);
    assert(!hantro_h264_ref_advance(&ctx.h264_refs));
    /* Exact ioctl wrappers preserve unrelated codecs and OUTPUT paths. */
    struct file file={&ctx};
    struct v4l2_requestbuffers req={cap.type, V4L2_MEMORY_DMABUF, 3};
    struct v4l2_create_buffers create={{cap.type}, V4L2_MEMORY_DMABUF, 3};
    struct v4l2_remove_buffers remove={cap.type, 1};
    assert(hantro_reqbufs(&file,NULL,&req)==-EINVAL && !delegates);
    assert(hantro_create_bufs(&file,NULL,&create)==-EINVAL && !delegates);
    req.memory=create.memory=V4L2_MEMORY_MMAP;
    assert(!hantro_reqbufs(&file,NULL,&req)); assert(!hantro_create_bufs(&file,NULL,&create));
    for (int stream=0; stream<2; stream++) {
        cap.streaming=stream==0; out.streaming=stream==1; int before=delegates;
        assert(hantro_create_bufs(&file,NULL,&create)==-EBUSY);
        assert(hantro_remove_bufs(&file,NULL,&remove)==-EBUSY); assert(before==delegates);
    }
    cap.streaming=true; req.count=0;
    assert(hantro_reqbufs(&file,NULL,&req)==-EBUSY); /* Pinned core rejects streaming REQBUFS(0). */
    create.count=remove.count=0;
    assert(!hantro_create_bufs(&file,NULL,&create)); assert(!hantro_remove_bufs(&file,NULL,&remove));
    cap.streaming=false; out.streaming=true;
    assert(!hantro_reqbufs(&file,NULL,&req)); /* CAPTURE streamoff already drained current job. */
    out.streaming=false; assert(!hantro_reqbufs(&file,NULL,&req));
    for (int bypass=0; bypass<3; bypass++) {
        req.count=create.count=remove.count=1; req.memory=create.memory=V4L2_MEMORY_DMABUF;
        fmt.codec_mode=bypass==0?HANTRO_MODE_JPEG_ENC:HANTRO_MODE_H264_ENC;
        ctx.is_encoder=bypass!=1; req.type=create.format.type=remove.type=bypass==2?out.type:cap.type;
        cap.streaming=out.streaming=true;
        assert(!hantro_create_bufs(&file,NULL,&create)); assert(!hantro_remove_bufs(&file,NULL,&remove));
        /* Generic REQBUFS may still reject streaming; wrapper must delegate. */
        int before=delegates; (void)hantro_reqbufs(&file,NULL,&req); assert(delegates==before+1);
        buffers[0].h264_ref.valid=true;
        slots[0]->vb2_queue=bypass==2?&out:&cap;
        hantro_h264_invalidate_buffer(slots[0]); assert(buffers[0].h264_ref.valid);
    }
    ctx.is_encoder=true; fmt.codec_mode=HANTRO_MODE_H264_ENC; slots[0]->vb2_queue=&cap;
    for (unsigned int i=0; i<3; i++) hantro_buf_cleanup(slots[i]);
    assert(allocations==releases && !held);
    puts("Actual lookup, buffer lifecycle, allocation ioctl and shared epoch boundary checks passed; vb2/DMA APIs are mocked, STREAMON/OFF are source invariants.");
    return 0;
}
