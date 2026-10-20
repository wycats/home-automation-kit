/* SPDX-License-Identifier: GPL-2.0 */
/* Host resource/workqueue doubles around exact candidate job functions. */
#include <assert.h>
#include <errno.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
typedef SRWLOCK test_lock;
#define LOCK_INIT SRWLOCK_INIT
static void lock(test_lock *l) { AcquireSRWLockExclusive(l); }
static void unlock(test_lock *l) { ReleaseSRWLockExclusive(l); }
#define THREAD_LOCAL __declspec(thread)
#else
#include <pthread.h>
typedef pthread_mutex_t test_lock;
#define LOCK_INIT PTHREAD_MUTEX_INITIALIZER
static void lock(test_lock *l) { assert(!pthread_mutex_lock(l)); }
static void unlock(test_lock *l) { assert(!pthread_mutex_unlock(l)); }
#define THREAD_LOCAL _Thread_local
#endif
static THREAD_LOCAL unsigned held;
static THREAD_LOCAL void (*unlock_hook)(void);
#define spin_lock_irqsave(l, f) do { (f) = 0; lock(l); assert(!held); held++; } while (0)
#define spin_unlock_irqrestore(l, f) do { (void)(f); assert(held == 1); held--; unlock(l); if (unlock_hook) unlock_hook(); } while (0)
#define lockdep_assert_held(l) do { (void)(l); assert(held); } while (0)
#define container_of(p, t, m) ((t *)((char *)(p) - offsetof(t, m)))
#define GFP_KERNEL 0
#define HANTRO_MODE_H264_ENC 1
#define HANTRO_MODE_JPEG_ENC 2
enum vb2_buffer_state { VB2_BUF_STATE_DONE, VB2_BUF_STATE_ERROR };
struct work_struct { void (*fn)(struct work_struct *); bool queued; };
struct delayed_work { struct work_struct work; };
#define INIT_WORK(w, f) do { (w)->fn = (f); (w)->queued = false; } while (0)
#define INIT_DELAYED_WORK(w, f) INIT_WORK(&(w)->work, f)
#define to_delayed_work(w) container_of(w, struct delayed_work, work)
struct hantro_ctx;
struct hantro_h264_job;
struct vb2_v4l2_buffer { struct { struct { void *req; } req_obj; } vb2_buf; };
struct variant { int num_clocks; };
struct hantro_dev {
    void *dev, *clocks;
    struct variant *variant;
    test_lock irqlock;
    struct hantro_h264_job *h264_job;
    bool h264_irq_mode, h264_mmio_live;
};
struct codec_ops {
    int (*run)(struct hantro_ctx *);
    void (*done)(struct hantro_ctx *);
    void (*reset)(struct hantro_ctx *);
};
struct format { int codec_mode; };
struct hantro_ctx {
    struct hantro_dev *dev;
    struct codec_ops *codec_ops;
    struct format *vpu_dst_fmt;
    bool is_encoder, h264_abort;
    struct hantro_h264_job *h264_job;
    int ctrl_handler;
    struct vb2_v4l2_buffer src, dst;
};

static int fail_alloc, fail_pm, fail_clock, fail_run, bad_payload;
static int allocations, frees, pm_live, clocks_live, completes, finishes, done_calls;
static int starts, resets, work_queues, timer_queues, drains;
static enum vb2_buffer_state finish_state;
static struct work_struct *queued_work;
static void (*drain_hook)(void);
static void (*finish_hook)(void);
static void (*prepare_hook)(struct hantro_ctx *);
static void (*after_start_hook)(void);
static void (*request_complete_hook)(void);
static bool wait_for_timeout;
#ifdef _WIN32
typedef HANDLE test_event;
static test_event event_new(void) { HANDLE e = CreateEvent(NULL, TRUE, FALSE, NULL); assert(e); return e; }
static void event_set(test_event e) { assert(SetEvent(e)); }
static void event_wait(test_event e) { assert(WaitForSingleObject(e, 10000) == WAIT_OBJECT_0); }
static void event_destroy(test_event e) { assert(CloseHandle(e)); }
#else
struct event_value { pthread_mutex_t mutex; pthread_cond_t cond; bool set; };
typedef struct event_value *test_event;
static test_event event_new(void) {
    test_event e = calloc(1, sizeof(*e)); assert(e);
    assert(!pthread_mutex_init(&e->mutex, NULL)); assert(!pthread_cond_init(&e->cond, NULL)); return e;
}
static void event_set(test_event e) {
    assert(!pthread_mutex_lock(&e->mutex)); e->set = true;
    assert(!pthread_cond_broadcast(&e->cond)); assert(!pthread_mutex_unlock(&e->mutex));
}
static void event_wait(test_event e) {
    assert(!pthread_mutex_lock(&e->mutex));
    while (!e->set) assert(!pthread_cond_wait(&e->cond, &e->mutex));
    assert(!pthread_mutex_unlock(&e->mutex));
}
static void event_destroy(test_event e) {
    assert(!pthread_cond_destroy(&e->cond)); assert(!pthread_mutex_destroy(&e->mutex)); free(e);
}
#endif
static test_event timeout_entered, timeout_release, timeout_returned, cancel_entered;
static void *kzalloc(size_t n, int flag)
{ (void)flag; if (fail_alloc) return NULL; allocations++; return calloc(1, n); }
static void kfree(void *p) { if (p) { frees++; free(p); } }
static int msecs_to_jiffies(int n) { return n; }
static bool schedule_work(struct work_struct *w)
{ assert(held && !w->queued && !queued_work); w->queued = true; queued_work = w; work_queues++; return true; }
static bool schedule_delayed_work(struct delayed_work *w, int delay)
{ assert(held && delay == 2000 && !w->work.queued); w->work.queued = true; timer_queues++; return true; }
static bool cancel_delayed_work_sync(struct delayed_work *w)
{
    assert(!held); drains++;
    if (wait_for_timeout) { event_set(cancel_entered); event_wait(timeout_returned); }
    /* Contract double: invoke an already-running loser before drain returns. */
    if (drain_hook) { void (*h)(void) = drain_hook; drain_hook = NULL; h(); }
    bool pending = w->work.queued; w->work.queued = false; return pending;
}
static struct vb2_v4l2_buffer *hantro_get_src_buf(struct hantro_ctx *ctx) { return &ctx->src; }
static struct vb2_v4l2_buffer *hantro_get_dst_buf(struct hantro_ctx *ctx) { return &ctx->dst; }
static int pm_runtime_resume_and_get(void *dev) { (void)dev; if (fail_pm) return -EIO; pm_live++; return 1; }
static void pm_runtime_put_autosuspend(void *dev) { (void)dev; assert(pm_live == 1); pm_live--; }
static int clk_bulk_enable(int n, void *clocks) { (void)n; (void)clocks; if (fail_clock) return -EIO; clocks_live++; return 0; }
static void clk_bulk_disable(int n, void *clocks) { (void)n; (void)clocks; assert(clocks_live == 1); clocks_live--; }
static void v4l2_m2m_buf_copy_metadata(struct vb2_v4l2_buffer *src, struct vb2_v4l2_buffer *dst)
{ assert(src && dst && !held); }
static void v4l2_ctrl_request_complete(void *req, int *handler) {
    assert(req && handler && !held); completes++;
    if (request_complete_hook) request_complete_hook();
}
static int rk3399_vpu_h264_enc_check_result(struct hantro_ctx *ctx) { assert(ctx && !held && clocks_live); return bad_payload; }
static void hantro_job_finish_no_pm(struct hantro_dev *vpu, struct hantro_ctx *ctx, enum vb2_buffer_state state)
{
    assert(!held && !vpu->h264_job && !ctx->h264_job && !vpu->h264_mmio_live);
    assert(!clocks_live && !pm_live); finishes++; finish_state = state;
    if (finish_hook) { void (*h)(void) = finish_hook; finish_hook = NULL; h(); }
}

#include "job_subject.inc"

static void reset_mmio(struct hantro_ctx *ctx)
{ assert(held && clocks_live && ctx->dev->h264_mmio_live); resets++; }
static void codec_done(struct hantro_ctx *ctx) { assert(ctx && !held && clocks_live); done_calls++; }
static void complete_now(void)
{
    struct work_struct *w = queued_work;
    assert(w && w->queued && !held); queued_work = NULL; w->queued = false; w->fn(w);
    /* w can be freed or a new job queued here; do not touch it. */
}
static void irq(struct hantro_dev *dev, enum vb2_buffer_state state)
{
    unsigned long flags;
    spin_lock_irqsave(&dev->irqlock, flags);
    hantro_h264_enc_irq_locked(dev, state);
    spin_unlock_irqrestore(&dev->irqlock, flags);
}
static int synthetic_run(struct hantro_ctx *ctx)
{
    struct hantro_dev *vpu = ctx->dev;
    unsigned long flags;
    if (prepare_hook) prepare_hook(ctx);
    if (fail_run) return -EINVAL;
    spin_lock_irqsave(&vpu->irqlock, flags);
    int ret = hantro_h264_enc_arm_locked(ctx);
    if (!ret) starts++; /* Real tail writes ENABLE under this same lock. */
    spin_unlock_irqrestore(&vpu->irqlock, flags);
    if (!ret && after_start_hook) after_start_hook();
    return ret;
}
static struct variant variant = {1};
static struct hantro_dev dev = { .variant = &variant, .irqlock = LOCK_INIT };
static struct codec_ops ops = { synthetic_run, codec_done, reset_mmio };
static struct format fmt = { HANTRO_MODE_H264_ENC };
static struct hantro_ctx a, b;
static void init_ctx(struct hantro_ctx *ctx)
{
    memset(ctx, 0, sizeof(*ctx)); ctx->dev = &dev; ctx->codec_ops = &ops;
    ctx->vpu_dst_fmt = &fmt; ctx->is_encoder = true; ctx->src.vb2_buf.req_obj.req = ctx;
}
static void clean(void)
{
    assert(!queued_work && !dev.h264_job && !pm_live && !clocks_live);
    assert(allocations == frees);
    fail_alloc = fail_pm = fail_clock = fail_run = bad_payload = 0;
    allocations = frees = completes = finishes = done_calls = 0;
    starts = resets = work_queues = timer_queues = drains = 0;
    drain_hook = finish_hook = after_start_hook = NULL; prepare_hook = NULL;
    request_complete_hook = NULL; wait_for_timeout = false;
    init_ctx(&a); init_ctx(&b);
}
static void timeout_job(struct hantro_h264_job *job) { job->watchdog.work.fn(&job->watchdog.work); }
static void timeout_active(void) { timeout_job(dev.h264_job); }
static void abort_a(void) { hantro_h264_abort(&a); }
static void inject_prepare_abort(struct hantro_ctx *ctx)
{ hantro_h264_abort(ctx); assert(!queued_work && ctx->h264_job->owner == HANTRO_H264_PREPARING); }
static void immediate_completion(void) { irq(&dev, VB2_BUF_STATE_DONE); complete_now(); }
static void start_a(void) { device_run(&a); }
static void start_b(void) { device_run(&b); }
static void release_dynamic(void) { free(dev.dev); dev.dev = NULL; }
static void pause_timeout_after_unlock(void)
{ unlock_hook = NULL; event_set(timeout_entered); event_wait(timeout_release); }

struct producer { int kind; struct hantro_h264_job *job; };
static void produce(struct producer *p)
{
    if (p->kind == 3) {
        unlock_hook = pause_timeout_after_unlock;
        timeout_job(p->job); event_set(timeout_returned);
    } else if (p->kind == 4) complete_now();
    else if (p->kind == 0) irq(&dev, VB2_BUF_STATE_DONE);
    else if (p->kind == 1) timeout_job(p->job);
    else hantro_h264_abort(&a);
}
#ifdef _WIN32
static DWORD WINAPI producer_thread(void *p) { produce(p); return 0; }
static HANDLE start_producer(struct producer *p) {
    HANDLE t = CreateThread(NULL, 0, producer_thread, p, 0, NULL); assert(t); return t;
}
static void join_producer(HANDLE t) { assert(WaitForSingleObject(t, 10000) == WAIT_OBJECT_0); assert(CloseHandle(t)); }
static void race(struct producer p[3])
{
    HANDLE t[3];
    for (int i = 0; i < 3; i++) { t[i] = CreateThread(NULL, 0, producer_thread, &p[i], 0, NULL); assert(t[i]); }
    assert(WaitForMultipleObjects(3, t, TRUE, INFINITE) == WAIT_OBJECT_0);
    for (int i = 0; i < 3; i++) assert(CloseHandle(t[i]));
}
#else
static void *producer_thread(void *p) { produce(p); return NULL; }
static pthread_t start_producer(struct producer *p) {
    pthread_t t; assert(!pthread_create(&t, NULL, producer_thread, p)); return t;
}
static void join_producer(pthread_t t) { assert(!pthread_join(t, NULL)); }
static void race(struct producer p[3])
{
    pthread_t t[3];
    for (int i = 0; i < 3; i++) assert(!pthread_create(&t[i], NULL, producer_thread, &p[i]));
    for (int i = 0; i < 3; i++) assert(!pthread_join(t[i], NULL));
}
#endif

int main(void)
{
    /* Allocation/PM/clock/run/abort-before-entry and abort-during-prepare. */
    for (int f = 0; f < 6; f++) {
        clean();
        if (f == 0) fail_alloc = 1;
        if (f == 1) fail_pm = 1;
        if (f == 2) fail_clock = 1;
        if (f == 3) fail_run = 1;
        if (f == 4) hantro_h264_abort(&a);
        if (f == 5) prepare_hook = inject_prepare_abort;
        device_run(&a);
        assert(completes == 1 && finishes == 1 && !starts && !timer_queues && !work_queues);
        assert(finish_state == VB2_BUF_STATE_ERROR);
    }
    /* All producer orderings, plus payload validation and late abort. */
    for (int winner = 0; winner < 3; winner++) {
        clean(); device_run(&a); struct hantro_h264_job *job = dev.h264_job;
        struct producer p[3] = {{0, job}, {1, job}, {2, job}};
        produce(&p[winner]);
        for (int i = 0; i < 3; i++) produce(&p[i]);
        assert(work_queues == 1 && job->owner == (winner == 0 ? HANTRO_H264_IRQ : winner == 1 ? HANTRO_H264_TIMEOUT : HANTRO_H264_ABORT));
        drain_hook = timeout_active; complete_now();
        assert(completes == 1 && finishes == 1 && drains == 1 && finish_state == VB2_BUF_STATE_ERROR);
        irq(&dev, VB2_BUF_STATE_DONE); assert(work_queues == 1);
    }
    clean(); device_run(&a); irq(&dev, VB2_BUF_STATE_DONE); bad_payload = 1; complete_now();
    assert(!done_calls && finish_state == VB2_BUF_STATE_ERROR);
    clean(); device_run(&a); irq(&dev, VB2_BUF_STATE_DONE); drain_hook = abort_a; complete_now();
    assert(!done_calls && finish_state == VB2_BUF_STATE_ERROR);
    clean(); device_run(&a); irq(&dev, VB2_BUF_STATE_DONE); request_complete_hook = abort_a; complete_now();
    assert(done_calls == 1 && finish_state == VB2_BUF_STATE_ERROR); /* Reference updates remain a separate gate. */
    /* A real host timeout callback is still active while completion drains it. */
    clean(); device_run(&a); wait_for_timeout = true;
    timeout_entered = event_new(); timeout_release = event_new();
    timeout_returned = event_new(); cancel_entered = event_new();
    struct producer timeout_p = {3, dev.h264_job}, completion_p = {4, NULL};
#ifdef _WIN32
    HANDLE timeout_thread = start_producer(&timeout_p);
#else
    pthread_t timeout_thread = start_producer(&timeout_p);
#endif
    event_wait(timeout_entered);
#ifdef _WIN32
    HANDLE completion_thread = start_producer(&completion_p);
#else
    pthread_t completion_thread = start_producer(&completion_p);
#endif
    event_wait(cancel_entered); assert(!finishes && !completes && !frees);
    event_set(timeout_release); join_producer(timeout_thread); join_producer(completion_thread);
    assert(finishes == 1 && completes == 1 && frees == 1);
    event_destroy(timeout_entered); event_destroy(timeout_release);
    event_destroy(timeout_returned); event_destroy(cancel_entered);
    /* Framework may start same/different ctx before old callback returns. */
    for (int same = 0; same < 2; same++) {
        clean(); device_run(&a); struct hantro_h264_job *old = dev.h264_job;
        irq(&dev, VB2_BUF_STATE_DONE); finish_hook = same ? start_a : start_b; complete_now();
        assert(dev.h264_job && dev.h264_job != old && starts == 2 && allocations == 2 && frees == 1);
        timeout_active(); complete_now(); assert(finishes == 2 && completes == 2 && frees == 2);
    }
    /* Completion before run returns; ctx freed at framework boundary. */
    clean();
    struct hantro_ctx *dynamic = malloc(sizeof(*dynamic)); assert(dynamic); init_ctx(dynamic);
    dev.dev = dynamic; finish_hook = release_dynamic; after_start_hook = immediate_completion;
    device_run(dynamic); assert(completes == 1 && finishes == 1 && !dev.dev);
    /* Real host threads compete using the exact candidate lock/claim paths. */
    for (int n = 0; n < 1000; n++) {
        clean(); device_run(&a);
        struct producer p[3] = {{0, dev.h264_job}, {1, dev.h264_job}, {2, dev.h264_job}};
        race(p); assert(work_queues == 1); complete_now();
        assert(completes == 1 && finishes == 1 && finish_state == VB2_BUF_STATE_ERROR);
    }
    clean();
    puts("15 deterministic actual-path scenarios, one active-timeout drain and 1000 three-producer host-thread races passed; kernel workqueue/MMIO/DMA are mocked.");
    return 0;
}
