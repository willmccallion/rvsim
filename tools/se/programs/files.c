/* Files on the host: create, write, seek, read back, stat, rename, unlink. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
    const char *dir = argc > 1 ? argv[1] : ".";
    char path[512], moved[512];
    snprintf(path, sizeof path, "%s/se_files_test.txt", dir);
    snprintf(moved, sizeof moved, "%s/se_files_moved.txt", dir);

    FILE *f = fopen(path, "w");
    if (!f) { perror("fopen"); return 1; }
    for (int i = 0; i < 100; i++) fprintf(f, "line %d\n", i);
    fclose(f);

    struct stat st;
    if (stat(path, &st) != 0) { perror("stat"); return 2; }
    printf("size %lld\n", (long long)st.st_size);

    f = fopen(path, "r");
    if (!f) { perror("fopen r"); return 3; }
    fseek(f, 0, SEEK_END);
    long end = ftell(f);
    fseek(f, -8, SEEK_END);
    char tail[16] = {0};
    size_t n = fread(tail, 1, 8, f);
    printf("end %ld tail %zu '%s'\n", end, n, tail);
    rewind(f);
    int lines = 0;
    char buf[64];
    while (fgets(buf, sizeof buf, f)) lines++;
    fclose(f);
    printf("lines %d\n", lines);

    if (rename(path, moved) != 0) { perror("rename"); return 4; }
    if (access(path, F_OK) == 0) { printf("old path still exists\n"); return 5; }
    if (unlink(moved) != 0) { perror("unlink"); return 6; }
    printf("ok\n");
    return 0;
}
