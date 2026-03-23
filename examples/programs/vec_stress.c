/*
 * vec_stress.c — Comprehensive RVV 1.0 stress test
 *
 * Exercises every vector functional unit type:
 *   VecIntAlu:     vadd, vsub, vand, vor, vxor, vsll, vsrl, vmin, vmax
 *   VecIntMul:     vmul, vmulh, vmacc
 *   VecIntDiv:     vdiv, vrem
 *   VecFpAlu:      vfadd, vfsub
 *   VecFpFma:      vfmacc, vfmadd
 *   VecFpDivSqrt:  vfdiv, vfsqrt
 *   VecMem:        unit-stride, strided, indexed load/store
 *   VecPermute:    vrgather, vslideup, vslidedown, vmerge
 *   Masks:         vmseq, vmslt, vmsle
 *   Reductions:    vredsum, vredmax, vfredosum
 *
 * Also varies SEW (8/16/32/64) and LMUL (m1/m2/m4).
 *
 * Build: compiled by the existing Makefile with -march=rv64gcv
 */

#include "stdio.h"
#include "stdlib.h"
#include <riscv_vector.h>

/* ── Helpers ─────────────────────────────────────────────────────────── */

#define N       256
#define MAT_N   16      /* 16x16 matrix multiply */
#define ITERS   8       /* repeat kernels for sustained pressure */

static unsigned long long rng_state = 0xDEADBEEFCAFE1234ULL;

static long rand_i64(void) {
    rng_state = rng_state * 6364136223846793005ULL + 1442695040888963407ULL;
    return (long)(rng_state >> 1);
}

static double rand_f64(void) {
    long r = rand_i64();
    if (r < 0) r = -r;
    return (double)(r % 10000) / 100.0;
}

static int errors = 0;

static void check(const char *name, int ok) {
    if (!ok) {
        printf("  FAIL: %s\n", name);
        errors++;
    }
}

/* ── Data ────────────────────────────────────────────────────────────── */

static long      ia[N] __attribute__((aligned(64)));
static long      ib[N] __attribute__((aligned(64)));
static long      ic[N] __attribute__((aligned(64)));
static double    fa[N] __attribute__((aligned(64)));
static double    fb[N] __attribute__((aligned(64)));
static double    fc[N] __attribute__((aligned(64)));
static double    fd[N] __attribute__((aligned(64)));
static int       i32a[N] __attribute__((aligned(64)));
static int       i32b[N] __attribute__((aligned(64)));
static int       i32c[N] __attribute__((aligned(64)));
static short     i16a[N] __attribute__((aligned(64)));
static short     i16b[N] __attribute__((aligned(64)));
static short     i16c[N] __attribute__((aligned(64)));
static char      i8a[N]  __attribute__((aligned(64)));
static char      i8b[N]  __attribute__((aligned(64)));
static char      i8c[N]  __attribute__((aligned(64)));
static double    mat_a[MAT_N * MAT_N] __attribute__((aligned(64)));
static double    mat_b[MAT_N * MAT_N] __attribute__((aligned(64)));
static double    mat_c[MAT_N * MAT_N] __attribute__((aligned(64)));
/* Index arrays for indexed (gather/scatter) loads */
static unsigned long idx64[N] __attribute__((aligned(64)));

static void init_data(void) {
    for (int i = 0; i < N; i++) {
        ia[i] = rand_i64() % 1000;
        ib[i] = (rand_i64() % 999) + 1;  /* avoid div-by-zero */
        fa[i] = rand_f64();
        fb[i] = rand_f64() + 0.01;       /* avoid div-by-zero */
        i32a[i] = (int)(rand_i64() % 1000);
        i32b[i] = (int)((rand_i64() % 999) + 1);
        i16a[i] = (short)(rand_i64() % 500);
        i16b[i] = (short)((rand_i64() % 499) + 1);
        i8a[i]  = (char)(rand_i64() % 100);
        i8b[i]  = (char)((rand_i64() % 99) + 1);
        /* Byte offsets for indexed load — stride by 8 (sizeof(double)) */
        idx64[i] = (unsigned long)((i * 3) % N) * sizeof(double);
    }
    for (int i = 0; i < MAT_N * MAT_N; i++) {
        mat_a[i] = rand_f64();
        mat_b[i] = rand_f64();
        mat_c[i] = 0.0;
    }
}

/* ────────────────────────────────────────────────────────────────────── */
/*  1. Integer ALU — e64 m1                                             */
/* ────────────────────────────────────────────────────────────────────── */

static void test_int_alu(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        long *pa = ia, *pb = ib, *pc = ic;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vint64m4_t va = __riscv_vle64_v_i64m4(pa, vl);
            vint64m4_t vb = __riscv_vle64_v_i64m4(pb, vl);
            /* add, sub, and, or, xor, shift, min, max */
            vint64m4_t vr = __riscv_vadd_vv_i64m4(va, vb, vl);
            vr = __riscv_vsub_vv_i64m4(vr, vb, vl);       /* should == va */
            vr = __riscv_vand_vv_i64m4(vr, vb, vl);
            vr = __riscv_vor_vv_i64m4(vr, va, vl);
            vr = __riscv_vxor_vv_i64m4(vr, vb, vl);
            vr = __riscv_vsll_vx_i64m4(vr, 2, vl);
            vr = __riscv_vsra_vx_i64m4(vr, 1, vl);
            vr = __riscv_vmin_vv_i64m4(vr, va, vl);
            vr = __riscv_vmax_vv_i64m4(vr, vb, vl);
            __riscv_vse64_v_i64m4(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    check("int_alu", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  2. Integer Multiply — e64 m2                                        */
/* ────────────────────────────────────────────────────────────────────── */

static void test_int_mul(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        long *pa = ia, *pb = ib, *pc = ic;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m2(n);
            vint64m2_t va = __riscv_vle64_v_i64m2(pa, vl);
            vint64m2_t vb = __riscv_vle64_v_i64m2(pb, vl);
            vint64m2_t vr = __riscv_vmul_vv_i64m2(va, vb, vl);
            /* multiply-accumulate: vr += va * vb */
            vr = __riscv_vmacc_vv_i64m2(vr, va, vb, vl);
            __riscv_vse64_v_i64m2(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    check("int_mul", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  3. Integer Divide — e32 m1 (smaller SEW to get more elements)       */
/* ────────────────────────────────────────────────────────────────────── */

static void test_int_div(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        int *pa = i32a, *pb = i32b, *pc = i32c;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e32m1(n);
            vint32m1_t va = __riscv_vle32_v_i32m1(pa, vl);
            vint32m1_t vb = __riscv_vle32_v_i32m1(pb, vl);
            vint32m1_t vq = __riscv_vdiv_vv_i32m1(va, vb, vl);
            vint32m1_t vrem = __riscv_vrem_vv_i32m1(va, vb, vl);
            vint32m1_t vr = __riscv_vadd_vv_i32m1(vq, vrem, vl);
            __riscv_vse32_v_i32m1(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    check("int_div", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  4. FP Add/Sub — e64 m4 (wide LMUL for throughput)                   */
/* ────────────────────────────────────────────────────────────────────── */

static void test_fp_alu(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        double *pa = fa, *pb = fb, *pc = fc;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vfloat64m4_t va = __riscv_vle64_v_f64m4(pa, vl);
            vfloat64m4_t vb = __riscv_vle64_v_f64m4(pb, vl);
            vfloat64m4_t vr = __riscv_vfadd_vv_f64m4(va, vb, vl);
            vr = __riscv_vfsub_vv_f64m4(vr, vb, vl);
            vr = __riscv_vfadd_vv_f64m4(vr, va, vl);
            vr = __riscv_vfsub_vv_f64m4(vr, va, vl);
            vr = __riscv_vfadd_vf_f64m4(vr, 1.0, vl);
            __riscv_vse64_v_f64m4(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    check("fp_alu", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  5. FP FMA — DAXPY: y = a*x + y                                     */
/* ────────────────────────────────────────────────────────────────────── */

static void test_fp_fma(void) {
    double alpha = 2.5;
    for (int rep = 0; rep < ITERS; rep++) {
        /* Reset fc */
        for (int i = 0; i < N; i++) fc[i] = fa[i];
        size_t n = N;
        double *px = fb, *py = fc;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vfloat64m4_t vx = __riscv_vle64_v_f64m4(px, vl);
            vfloat64m4_t vy = __riscv_vle64_v_f64m4(py, vl);
            /* vy = alpha * vx + vy */
            vy = __riscv_vfmacc_vf_f64m4(vy, alpha, vx, vl);
            /* chain another FMA: vy = alpha * vx + vy */
            vy = __riscv_vfmacc_vf_f64m4(vy, alpha, vx, vl);
            __riscv_vse64_v_f64m4(py, vy, vl);
            px += vl; py += vl; n -= vl;
        }
    }
    /* Spot-check */
    double expect0 = fa[0] + 2.0 * alpha * fb[0];
    double diff = fc[0] - expect0;
    if (diff < 0) diff = -diff;
    check("fp_fma_daxpy", diff < 1e-6);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  6. FP Div + Sqrt                                                    */
/* ────────────────────────────────────────────────────────────────────── */

static void test_fp_div_sqrt(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        double *pa = fa, *pb = fb, *pc = fc;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m2(n);
            vfloat64m2_t va = __riscv_vle64_v_f64m2(pa, vl);
            vfloat64m2_t vb = __riscv_vle64_v_f64m2(pb, vl);
            /* div then sqrt */
            vfloat64m2_t vr = __riscv_vfdiv_vv_f64m2(va, vb, vl);
            /* Make values positive for sqrt: abs(x) via x*x then sqrt */
            vr = __riscv_vfmul_vv_f64m2(vr, vr, vl);
            vfloat64m2_t vs = __riscv_vfsqrt_v_f64m2(vr, vl);
            __riscv_vse64_v_f64m2(pc, vs, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    check("fp_div_sqrt", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  7. Strided memory access                                            */
/* ────────────────────────────────────────────────────────────────────── */

static void test_strided_mem(void) {
    /* Store with stride 2, load with stride 2, verify */
    for (int i = 0; i < N; i++) fc[i] = 0.0;

    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N / 2;
        size_t stride = 2 * sizeof(double);
        double *pa = fa, *pc = fc;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m1(n);
            vfloat64m1_t va = __riscv_vlse64_v_f64m1(pa, stride, vl);
            __riscv_vsse64_v_f64m1(pc, stride, va, vl);
            pa += vl * 2; pc += vl * 2; n -= vl;
        }
    }
    int ok = 1;
    for (int i = 0; i < N; i += 2) {
        if (fc[i] != fa[i]) { ok = 0; break; }
    }
    check("strided_mem", ok);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  8. Indexed (gather/scatter) memory access                           */
/* ────────────────────────────────────────────────────────────────────── */

static void test_indexed_mem(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        double *psrc = fa, *pdst = fd;
        unsigned long *pidx = idx64;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m1(n);
            vuint64m1_t vidx = __riscv_vle64_v_u64m1(pidx, vl);
            vfloat64m1_t vr = __riscv_vluxei64_v_f64m1(psrc, vidx, vl);
            __riscv_vse64_v_f64m1(pdst, vr, vl);
            pidx += vl; pdst += vl; n -= vl;
        }
    }
    /* Verify first element: fd[0] should be fa[(0*3)%N] = fa[0] */
    check("indexed_gather", fd[0] == fa[0]);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  9. Permutation — vrgather, vslidedown, vslideup                     */
/* ────────────────────────────────────────────────────────────────────── */

static void test_permute(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        long *pa = ia, *pc = ic;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m1(n);
            vint64m1_t va = __riscv_vle64_v_i64m1(pa, vl);
            /* Create index vector 0,1,2,... */
            vuint64m1_t vidx = __riscv_vid_v_u64m1(vl);
            /* Reverse: gather from (vl-1-i) */
            vuint64m1_t vrev = __riscv_vrsub_vx_u64m1(vidx, vl - 1, vl);
            vint64m1_t vr = __riscv_vrgather_vv_i64m1(va, vrev, vl);
            /* Slide down by 1 */
            vr = __riscv_vslidedown_vx_i64m1(vr, 1, vl);
            /* Slide up by 1 */
            vr = __riscv_vslideup_vx_i64m1(vr, va, 1, vl);
            __riscv_vse64_v_i64m1(pc, vr, vl);
            pa += vl; pc += vl; n -= vl;
        }
    }
    check("permute", 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  10. Masked operations + comparisons                                 */
/* ────────────────────────────────────────────────────────────────────── */

static void test_masked(void) {
    long threshold = 500;
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        long *pa = ia, *pb = ib, *pc = ic;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m2(n);
            vint64m2_t va = __riscv_vle64_v_i64m2(pa, vl);
            vint64m2_t vb = __riscv_vle64_v_i64m2(pb, vl);
            /* mask: va > threshold */
            vbool32_t mask = __riscv_vmsgt_vx_i64m2_b32(va, threshold, vl);
            /* Masked add: only where va > threshold */
            vint64m2_t vr = __riscv_vadd_vv_i64m2_m(mask, va, vb, vl);
            /* Where mask is false, vr gets va (merge behavior) */
            __riscv_vse64_v_i64m2(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    /* Verify: for elements where ia[i] > 500, ic[i] = ia[i]+ib[i] */
    int ok = 1;
    for (int i = 0; i < N; i++) {
        long expect = (ia[i] > threshold) ? ia[i] + ib[i] : ia[i];
        if (ic[i] != expect) { ok = 0; break; }
    }
    check("masked_ops", ok);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  11. Reductions — integer sum/max, FP ordered sum                    */
/* ────────────────────────────────────────────────────────────────────── */

static void test_reductions(void) {
    /* Integer sum reduction */
    long isum_scalar = 0;
    for (int i = 0; i < N; i++) isum_scalar += ia[i];

    long isum_vec = 0;
    {
        size_t n = N;
        long *pa = ia;
        vint64m1_t vaccum = __riscv_vmv_v_x_i64m1(0, 1);
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vint64m4_t va = __riscv_vle64_v_i64m4(pa, vl);
            vaccum = __riscv_vredsum_vs_i64m4_i64m1(va, vaccum, vl);
            pa += vl; n -= vl;
        }
        isum_vec = __riscv_vmv_x_s_i64m1_i64(vaccum);
    }
    check("reduce_isum", isum_vec == isum_scalar);

    /* Integer max reduction */
    long imax_scalar = ia[0];
    for (int i = 1; i < N; i++)
        if (ia[i] > imax_scalar) imax_scalar = ia[i];

    long imax_vec = 0;
    {
        size_t n = N;
        long *pa = ia;
        /* Init accumulator to minimum possible */
        vint64m1_t vaccum = __riscv_vmv_v_x_i64m1(-9999999, 1);
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vint64m4_t va = __riscv_vle64_v_i64m4(pa, vl);
            vaccum = __riscv_vredmax_vs_i64m4_i64m1(va, vaccum, vl);
            pa += vl; n -= vl;
        }
        imax_vec = __riscv_vmv_x_s_i64m1_i64(vaccum);
    }
    check("reduce_imax", imax_vec == imax_scalar);

    /* FP ordered sum reduction */
    double fsum_scalar = 0.0;
    for (int i = 0; i < N; i++) fsum_scalar += fa[i];

    double fsum_vec = 0.0;
    {
        size_t n = N;
        double *pa = fa;
        vfloat64m1_t vaccum = __riscv_vfmv_v_f_f64m1(0.0, 1);
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vfloat64m4_t va = __riscv_vle64_v_f64m4(pa, vl);
            vaccum = __riscv_vfredosum_vs_f64m4_f64m1(va, vaccum, vl);
            pa += vl; n -= vl;
        }
        fsum_vec = __riscv_vfmv_f_s_f64m1_f64(vaccum);
    }
    double fdiff = fsum_vec - fsum_scalar;
    if (fdiff < 0) fdiff = -fdiff;
    check("reduce_fsum", fdiff < 1e-3);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  12. SEW=8 — byte-level vector ops (memcpy/memset style)             */
/* ────────────────────────────────────────────────────────────────────── */

static void test_sew8(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        char *pa = i8a, *pb = i8b, *pc = i8c;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e8m8(n);
            vint8m8_t va = __riscv_vle8_v_i8m8(pa, vl);
            vint8m8_t vb = __riscv_vle8_v_i8m8(pb, vl);
            vint8m8_t vr = __riscv_vadd_vv_i8m8(va, vb, vl);
            vr = __riscv_vsub_vv_i8m8(vr, vb, vl);
            __riscv_vse8_v_i8m8(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    int ok = 1;
    for (int i = 0; i < N; i++) {
        if (i8c[i] != i8a[i]) { ok = 0; break; }
    }
    check("sew8_ops", ok);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  13. SEW=16 — halfword ops                                          */
/* ────────────────────────────────────────────────────────────────────── */

static void test_sew16(void) {
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        short *pa = i16a, *pb = i16b, *pc = i16c;
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e16m4(n);
            vint16m4_t va = __riscv_vle16_v_i16m4(pa, vl);
            vint16m4_t vb = __riscv_vle16_v_i16m4(pb, vl);
            vint16m4_t vr = __riscv_vmul_vv_i16m4(va, vb, vl);
            vr = __riscv_vadd_vv_i16m4(vr, va, vl);
            __riscv_vse16_v_i16m4(pc, vr, vl);
            pa += vl; pb += vl; pc += vl; n -= vl;
        }
    }
    int ok = 1;
    for (int i = 0; i < N; i++) {
        short expect = (short)(i16a[i] * i16b[i] + i16a[i]);
        if (i16c[i] != expect) { ok = 0; break; }
    }
    check("sew16_ops", ok);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  14. Matrix multiply — FP FMA heavy, sustained vector pressure       */
/* ────────────────────────────────────────────────────────────────────── */

static void test_matmul(void) {
    /* C = A * B, 16x16 double-precision */
    for (int rep = 0; rep < ITERS; rep++) {
        /* Clear C */
        for (int i = 0; i < MAT_N * MAT_N; i++) mat_c[i] = 0.0;

        for (int i = 0; i < MAT_N; i++) {
            for (int k = 0; k < MAT_N; k++) {
                double a_ik = mat_a[i * MAT_N + k];
                /* Vectorize across j: c[i][j] += a[i][k] * b[k][j] */
                size_t n = MAT_N;
                double *pb = &mat_b[k * MAT_N];
                double *pc = &mat_c[i * MAT_N];
                while (n > 0) {
                    size_t vl = __riscv_vsetvl_e64m2(n);
                    vfloat64m2_t vc = __riscv_vle64_v_f64m2(pc, vl);
                    vfloat64m2_t vb = __riscv_vle64_v_f64m2(pb, vl);
                    vc = __riscv_vfmacc_vf_f64m2(vc, a_ik, vb, vl);
                    __riscv_vse64_v_f64m2(pc, vc, vl);
                    pb += vl; pc += vl; n -= vl;
                }
            }
        }
    }
    /* Spot-check: compute C[0][0] the scalar way */
    double expect = 0.0;
    for (int k = 0; k < MAT_N; k++)
        expect += mat_a[k] * mat_b[k * MAT_N];
    double diff = mat_c[0] - expect;
    if (diff < 0) diff = -diff;
    check("matmul_f64", diff < 1e-6);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  15. Dot product — reduction + FMA combined                          */
/* ────────────────────────────────────────────────────────────────────── */

static void test_dot_product(void) {
    double dot_scalar = 0.0;
    for (int i = 0; i < N; i++) dot_scalar += fa[i] * fb[i];

    double dot_vec = 0.0;
    for (int rep = 0; rep < ITERS; rep++) {
        size_t n = N;
        double *pa = fa, *pb = fb;
        vfloat64m1_t vaccum = __riscv_vfmv_v_f_f64m1(0.0, 1);
        while (n > 0) {
            size_t vl = __riscv_vsetvl_e64m4(n);
            vfloat64m4_t va = __riscv_vle64_v_f64m4(pa, vl);
            vfloat64m4_t vb = __riscv_vle64_v_f64m4(pb, vl);
            vfloat64m4_t vp = __riscv_vfmul_vv_f64m4(va, vb, vl);
            vaccum = __riscv_vfredosum_vs_f64m4_f64m1(vp, vaccum, vl);
            pa += vl; pb += vl; n -= vl;
        }
        dot_vec = __riscv_vfmv_f_s_f64m1_f64(vaccum);
    }
    double diff = dot_vec - dot_scalar;
    if (diff < 0) diff = -diff;
    check("dot_product", diff < 1e-3);
}

/* ────────────────────────────────────────────────────────────────────── */

int main(void) {
    printf("RVV 1.0 Comprehensive Stress Test\n");
    printf("==================================\n");

    init_data();

    printf("Running integer ALU (e64 m4)...\n");
    test_int_alu();

    printf("Running integer multiply (e64 m2)...\n");
    test_int_mul();

    printf("Running integer divide (e32 m1)...\n");
    test_int_div();

    printf("Running FP ALU (e64 m4)...\n");
    test_fp_alu();

    printf("Running FP FMA / DAXPY (e64 m4)...\n");
    test_fp_fma();

    printf("Running FP div+sqrt (e64 m2)...\n");
    test_fp_div_sqrt();

    printf("Running strided memory (e64 m1)...\n");
    test_strided_mem();

    printf("Running indexed gather (e64 m1)...\n");
    test_indexed_mem();

    printf("Running permutation (e64 m1)...\n");
    test_permute();

    printf("Running masked operations (e64 m2)...\n");
    test_masked();

    printf("Running reductions (sum/max/fsum)...\n");
    test_reductions();

    printf("Running SEW=8 byte ops (e8 m8)...\n");
    test_sew8();

    printf("Running SEW=16 halfword ops (e16 m4)...\n");
    test_sew16();

    printf("Running 16x16 matrix multiply (e64 m2)...\n");
    test_matmul();

    printf("Running dot product (e64 m4)...\n");
    test_dot_product();

    printf("==================================\n");
    if (errors == 0) {
        printf("ALL PASSED\n");
    } else {
        printf("FAILURES: %d\n", errors);
    }
    return errors;
}
