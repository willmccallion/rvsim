#include "stdio.h"

double sqrt(double n) {
  double res;
  asm("fsqrt.d %0, %1" : "=f"(res) : "f"(n));
  return res;
}

typedef struct {
  double x, y, z;
} Vec3;

Vec3 v_add(Vec3 a, Vec3 b) { return (Vec3){a.x + b.x, a.y + b.y, a.z + b.z}; }
Vec3 v_sub(Vec3 a, Vec3 b) { return (Vec3){a.x - b.x, a.y - b.y, a.z - b.z}; }
Vec3 v_mul(Vec3 a, double s) { return (Vec3){a.x * s, a.y * s, a.z * s}; }
double v_dot(Vec3 a, Vec3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }

Vec3 v_norm(Vec3 a) {
  double len = sqrt(v_dot(a, a));
  if (len == 0.0)
    return (Vec3){0, 0, 0};
  return v_mul(a, 1.0 / len);
}

typedef struct {
  Vec3 center;
  double radius;
} Sphere;

// Returns distance t, or -1.0 if no intersection
double intersect_sphere(Vec3 ro, Vec3 rd, Sphere s, int debug) {
  Vec3 oc = v_sub(ro, s.center);

  double b = v_dot(oc, rd);
  double c = v_dot(oc, oc) - s.radius * s.radius;
  double h = b * b - c;

  if (debug) {
    printf("  [ISECT] Sphere Center: %f\n", s.center.z);
    printf("  [ISECT] oc: %f, %f, %f\n", oc.x, oc.y, oc.z);
    printf("  [ISECT] b (dot(oc, rd)): %f\n", b);
    printf("  [ISECT] c: %f\n", c);
    printf("  [ISECT] h (b*b - c): %f\n", h);
  }

  if (h < 0.0)
    return -1.0;

  h = sqrt(h);
  if (debug) {
    printf("  [ISECT] sqrt(h): %f\n", h);
    printf("  [ISECT] Result t: %f\n", -b - h);
  }

  return -b - h;
}

int main() {
  printf("RISC-V Hardware Raytracer (Double Precision)\n");

  int w = 64;
  int h = 32;
  double aspect = (double)w / (double)h;
  double pixel_corr = 0.5;

  Sphere spheres[3] = {
      {{0.0, 0.0, -5.0}, 1.0},  // Center Sphere
      {{1.5, 0.5, -4.0}, 0.5},  // Right Sphere
      {{-1.5, -0.5, -4.5}, 0.5} // Left Sphere
  };

  Vec3 light = v_norm((Vec3){-1.0, -1.0, -1.0});
  char ramp[] = " .:-=+*#%@";

  printf("Rendering Scene...\n");

  for (int y = 0; y < h; y++) {
    for (int x = 0; x < w; x++) {

      double uv_x = (2.0 * x / w - 1.0) * aspect * pixel_corr;
      double uv_y = (1.0 - 2.0 * y / h);

      Vec3 ro = {0.0, 0.0, 0.0};
      Vec3 rd = v_norm((Vec3){uv_x, uv_y, -1.0});

      double t_min = 1e9;
      int hit_idx = -1;

      for (int i = 0; i < 3; i++) {
        // Pass 0 for debug to disable prints inside intersect
        double t = intersect_sphere(ro, rd, spheres[i], 0);
        if (t > 0.0 && t < t_min) {
          t_min = t;
          hit_idx = i;
        }
      }

      if (hit_idx != -1) {
        Vec3 p = v_add(ro, v_mul(rd, t_min));
        Vec3 n = v_norm(v_sub(p, spheres[hit_idx].center));

        double diff = v_dot(n, v_mul(light, -1.0));
        if (diff < 0.0)
          diff = 0.0;

        diff += 0.1;
        if (diff > 1.0)
          diff = 1.0;

        int c_idx = (int)(diff * 8.0);
        putchar(ramp[c_idx]);
      } else {
        putchar(' ');
      }
    }
    putchar('\n');
  }

  printf("Done.\n");
  return 0;
}
