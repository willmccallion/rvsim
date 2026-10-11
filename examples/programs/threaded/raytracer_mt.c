/* A raytracer whose rows are shared out between threads: thread t renders
 * rows t, t + T, t + 2T, ... of a scene of spheres over a checkered floor,
 * with shadows and a reflection bounce, into one frame. The first thread
 * renders its own rows, joins the others, then prints the frame and the
 * cycles the render took. Give it a hart per thread:
 *
 *     rvsim raytracer_mt.elf --harts 4 -- 4
 *
 * With no argument it starts a thread for every CPU sysconf reports. */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "bench.h"

#define WIDTH 96
#define HEIGHT 40
#define MAX_THREADS 64
#define MAX_DEPTH 2

typedef struct {
  double x, y, z;
} Vec3;

typedef struct {
  Vec3 center;
  double radius;
  double shine; /* how much of a reflection it shows, 0 to 1 */
} Sphere;

static const Sphere spheres[] = {
    {{0.0, 0.0, -5.0}, 1.0, 0.4},   {{1.8, -0.4, -4.2}, 0.6, 0.0},
    {{-1.9, -0.3, -4.6}, 0.7, 0.2}, {{0.7, -0.75, -3.4}, 0.25, 0.0},
    {{-0.6, 0.9, -6.5}, 0.5, 0.6},  {{2.6, 1.1, -7.0}, 0.9, 0.0},
};
#define SPHERES (int)(sizeof spheres / sizeof spheres[0])
#define FLOOR_Y -1.0
#define NO_HIT -1
#define FLOOR -2

static const char ramp[] = " .:-=+*#%@";
static char frame[HEIGHT][WIDTH];

static double root(double n) {
  double r;
  asm("fsqrt.d %0, %1" : "=f"(r) : "f"(n));
  return r;
}

static Vec3 add(Vec3 a, Vec3 b) { return (Vec3){a.x + b.x, a.y + b.y, a.z + b.z}; }
static Vec3 sub(Vec3 a, Vec3 b) { return (Vec3){a.x - b.x, a.y - b.y, a.z - b.z}; }
static Vec3 scale(Vec3 a, double s) { return (Vec3){a.x * s, a.y * s, a.z * s}; }
static double dot(Vec3 a, Vec3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
static Vec3 norm(Vec3 a) { return scale(a, 1.0 / root(dot(a, a))); }

static const Vec3 to_light = {0.5773502691896258, 0.5773502691896258, 0.5773502691896258};

/* The distance along the ray to sphere s, or a negative number if it misses. */
static double hit_sphere(Vec3 ro, Vec3 rd, const Sphere *s) {
  Vec3 oc = sub(ro, s->center);
  double b = dot(oc, rd);
  double h = b * b - (dot(oc, oc) - s->radius * s->radius);
  if (h < 0.0)
    return -1.0;
  h = root(h);
  double t = -b - h;
  return t > 1e-6 ? t : -b + h;
}

/* What the ray meets first: a sphere's index, FLOOR or NO_HIT, and where. */
static int nearest(Vec3 ro, Vec3 rd, double *t_out) {
  int hit = NO_HIT;
  double t_min = 1e9;
  for (int i = 0; i < SPHERES; i++) {
    double t = hit_sphere(ro, rd, &spheres[i]);
    if (t > 1e-6 && t < t_min) {
      t_min = t;
      hit = i;
    }
  }
  if (rd.y < 0.0) {
    double t = (FLOOR_Y - ro.y) / rd.y;
    if (t > 1e-6 && t < t_min) {
      t_min = t;
      hit = FLOOR;
    }
  }
  *t_out = t_min;
  return hit;
}

static int in_shadow(Vec3 p) {
  double t;
  return nearest(p, to_light, &t) >= 0;
}

/* The brightness, 0 to 1, the ray sees. */
static double trace(Vec3 ro, Vec3 rd, int depth) {
  double t;
  int hit = nearest(ro, rd, &t);
  if (hit == NO_HIT)
    return 0.0;
  Vec3 p = add(ro, scale(rd, t));
  Vec3 n;
  double base, shine;
  if (hit == FLOOR) {
    n = (Vec3){0.0, 1.0, 0.0};
    long cx = (long)(p.x + 1000.0), cz = (long)(p.z + 1000.0);
    base = (cx + cz) % 2 ? 0.85 : 0.35;
    shine = 0.1;
  } else {
    n = norm(sub(p, spheres[hit].center));
    base = 0.9;
    shine = spheres[hit].shine;
  }
  Vec3 lifted = add(p, scale(n, 1e-6));
  double diffuse = dot(n, to_light);
  if (diffuse < 0.0)
    diffuse = 0.0;
  if (diffuse > 0.0 && in_shadow(lifted))
    diffuse *= 0.15;
  double colour = base * (0.12 + 0.88 * diffuse);
  if (shine > 0.0 && depth < MAX_DEPTH) {
    Vec3 mirrored = sub(rd, scale(n, 2.0 * dot(rd, n)));
    colour = colour * (1.0 - shine) + shine * trace(lifted, mirrored, depth + 1);
  }
  return colour;
}

static void render_rows(int first, int step) {
  double aspect = (double)WIDTH / HEIGHT * 0.5;
  for (int y = first; y < HEIGHT; y += step) {
    for (int x = 0; x < WIDTH; x++) {
      Vec3 rd = norm((Vec3){(2.0 * x / WIDTH - 1.0) * aspect, 1.0 - 2.0 * y / HEIGHT, -1.5});
      double c = trace((Vec3){0.0, 0.0, 0.0}, rd, 0);
      int level = (int)(c * (sizeof ramp - 1));
      frame[y][x] = ramp[level < 0 ? 0 : level > (int)sizeof ramp - 2 ? (int)sizeof ramp - 2 : level];
    }
  }
}

struct share {
  int first, step;
};

static void *render_share(void *arg) {
  const struct share *share = arg;
  render_rows(share->first, share->step);
  return NULL;
}

int main(int argc, char **argv) {
  int threads = argc > 1 ? atoi(argv[1]) : (int)sysconf(_SC_NPROCESSORS_ONLN);
  if (threads < 1)
    threads = 1;
  if (threads > MAX_THREADS)
    threads = MAX_THREADS;
  printf("Rendering %dx%d on %d thread%s...\n", WIDTH, HEIGHT, threads, threads == 1 ? "" : "s");

  pthread_t workers[MAX_THREADS];
  struct share shares[MAX_THREADS];
  unsigned long start = read_cycles();
  for (int t = 1; t < threads; t++) {
    shares[t] = (struct share){t, threads};
    int rc = pthread_create(&workers[t], NULL, render_share, &shares[t]);
    if (rc != 0) {
      printf("pthread_create: %s; give the run a hart per thread\n", strerror(rc));
      return 1;
    }
  }
  render_rows(0, threads);
  for (int t = 1; t < threads; t++)
    pthread_join(workers[t], NULL);
  unsigned long end = read_cycles();

  unsigned long checksum = 0;
  for (int y = 0; y < HEIGHT; y++) {
    printf("%.*s\n", WIDTH, frame[y]);
    for (int x = 0; x < WIDTH; x++)
      checksum = checksum * 31 + (unsigned char)frame[y][x];
  }
  printf("Checksum: %lu\n", checksum);
  printf("Render Cycles: %lu\n", end - start);
  return 0;
}
