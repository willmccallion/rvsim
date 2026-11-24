/* Bandwidth kernels for bandwidth_probe.py, one per build: KERNEL selects
 * COPY, FILL or READ over FOOTPRINT bytes, run PASSES times after a
 * warm-up pass. COPY and FILL are tinymembench's aligned_block_copy and
 * aligned_block_fill (util.c), the loops behind its "C copy" and "C fill"
 * results, kept as written; READ sums the buffer with eight loads a line
 * into four accumulators. tinymembench's notice follows. */
/*
 * Copyright © 2011-2016 Siarhei Siamashka <siarhei.siamashka@gmail.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, sublicense,
 * and/or sell copies of the Software, and to permit persons to whom the
 * Software is furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice (including the next
 * paragraph) shall be included in all copies or substantial portions of the
 * Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
 * DEALINGS IN THE SOFTWARE.
 */

typedef long int64_t;
#define COPY 1
#define FILL 2
#define READ 3
#define WORDS (FOOTPRINT / 8)

static int64_t src[WORDS] __attribute__((aligned(64)));
#if KERNEL == COPY
static int64_t dst[WORDS] __attribute__((aligned(64)));
#endif
static volatile int64_t sink;

void aligned_block_copy(int64_t * __restrict dst_,
                        int64_t * __restrict src,
                        int                  size)
{
    volatile int64_t *dst = dst_;
    int64_t t1, t2, t3, t4;
    while ((size -= 64) >= 0)
    {
        t1 = *src++;
        t2 = *src++;
        t3 = *src++;
        t4 = *src++;
        *dst++ = t1;
        *dst++ = t2;
        *dst++ = t3;
        *dst++ = t4;
        t1 = *src++;
        t2 = *src++;
        t3 = *src++;
        t4 = *src++;
        *dst++ = t1;
        *dst++ = t2;
        *dst++ = t3;
        *dst++ = t4;
    }
}

void aligned_block_fill(int64_t * __restrict dst_,
                        int64_t * __restrict src,
                        int                  size)
{
    volatile int64_t *dst = dst_;
    int64_t data = *src;
    while ((size -= 64) >= 0)
    {
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
        *dst++ = data;
    }
}

static int64_t block_read(const int64_t *p, int size)
{
    int64_t a0 = 0, a1 = 0, a2 = 0, a3 = 0;
    while ((size -= 64) >= 0)
    {
        a0 += p[0];
        a1 += p[1];
        a2 += p[2];
        a3 += p[3];
        a0 += p[4];
        a1 += p[5];
        a2 += p[6];
        a3 += p[7];
        p += 8;
    }
    return a0 + a1 + a2 + a3;
}

static void run_pass(void)
{
#if KERNEL == COPY
    aligned_block_copy(dst, src, FOOTPRINT);
#elif KERNEL == FILL
    aligned_block_fill(src, src, FOOTPRINT);
#else
    sink = block_read(src, FOOTPRINT);
#endif
}

int main(void)
{
    for (long i = 0; i < WORDS; i++)
        src[i] = i;
    for (long pass = 0; pass <= PASSES; pass++)
        run_pass();
    return 0;
}
