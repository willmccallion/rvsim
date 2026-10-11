/* Threads: a contended mutex, a condition variable, a barrier, thread-local
 * storage, thread ids, joins with return values, a second round of threads
 * after the first has exited, and a timed wait nobody signals. Only the
 * first thread prints, after joining, so the output does not depend on how
 * the threads interleave. */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define WORKERS 4
#define ROUNDS 500

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t all_arrived = PTHREAD_COND_INITIALIZER;
static pthread_cond_t never = PTHREAD_COND_INITIALIZER;
static pthread_barrier_t barrier;
static long counter;
static int arrived;
static __thread int tls_value;

struct work {
    int index;
    long sum;
    pid_t tid;
    int saw_own_tls;
};

static void *worker(void *arg) {
    struct work *w = arg;
    tls_value = 100 * w->index;
    w->tid = (pid_t)syscall(SYS_gettid);
    for (int i = 0; i < ROUNDS; i++) {
        pthread_mutex_lock(&lock);
        counter++;
        pthread_mutex_unlock(&lock);
    }
    pthread_mutex_lock(&lock);
    if (++arrived == WORKERS)
        pthread_cond_broadcast(&all_arrived);
    while (arrived < WORKERS)
        pthread_cond_wait(&all_arrived, &lock);
    pthread_mutex_unlock(&lock);
    pthread_barrier_wait(&barrier);
    long sum = 0;
    for (int i = 0; i <= 1000; i++)
        sum += (long)i * (w->index + 1);
    w->sum = sum;
    w->saw_own_tls = tls_value == 100 * w->index;
    if (w->index == WORKERS - 1)
        pthread_exit((void *)(long)(10 * (w->index + 1)));
    return (void *)(long)(w->index + 1);
}

static void *square(void *arg) {
    long n = (long)arg;
    return (void *)(n * n);
}

int main(void) {
    pthread_t threads[WORKERS];
    struct work work[WORKERS];
    tls_value = -1;
    pthread_barrier_init(&barrier, NULL, WORKERS);
    for (int i = 0; i < WORKERS; i++) {
        work[i] = (struct work){.index = i};
        int rc = pthread_create(&threads[i], NULL, worker, &work[i]);
        if (rc != 0) {
            printf("pthread_create: %s\n", strerror(rc));
            return 1;
        }
    }
    int ids_differ = 1;
    for (int i = 0; i < WORKERS; i++) {
        void *ret;
        pthread_join(threads[i], &ret);
        printf("worker %d returned %ld, sum %ld, own TLS %s\n", i, (long)ret, work[i].sum,
               work[i].saw_own_tls ? "yes" : "no");
        if (work[i].tid == getpid())
            ids_differ = 0;
        for (int j = 0; j < i; j++)
            if (work[j].tid == work[i].tid)
                ids_differ = 0;
    }
    printf("counter %ld of %d\n", counter, WORKERS * ROUNDS);
    printf("thread ids distinct from each other and the process: %s\n", ids_differ ? "yes" : "no");
    printf("first thread's TLS untouched: %s\n", tls_value == -1 ? "yes" : "no");

    long squares = 0;
    for (long n = 1; n <= WORKERS; n++) {
        pthread_t t;
        void *ret;
        if (pthread_create(&t, NULL, square, (void *)n) != 0)
            return 2;
        pthread_join(t, &ret);
        squares += (long)ret;
    }
    printf("second round, one at a time: %ld\n", squares);

    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline);
    deadline.tv_nsec += 2000000;
    if (deadline.tv_nsec >= 1000000000) {
        deadline.tv_sec += 1;
        deadline.tv_nsec -= 1000000000;
    }
    pthread_mutex_lock(&lock);
    int rc = pthread_cond_timedwait(&never, &lock, &deadline);
    pthread_mutex_unlock(&lock);
    printf("timed wait: %s\n", rc == ETIMEDOUT ? "timed out" : strerror(rc));
    return counter == WORKERS * ROUNDS ? 0 : 3;
}
