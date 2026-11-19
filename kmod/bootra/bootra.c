// bootra: boot read-ahead for the EC-NB01-CE laptop (2026-11-19).
// Reads a list of "path<TAB>offset<TAB>length" lines and asks the kernel to pull those
// ranges into the page cache ahead of the processes that will need them during boot.
// Files that do not exist yet (apex / data not mounted) are retried for a while.
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>

struct ent { char *path; long long off, len; int done; };

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: bootra <list> [max-wait-seconds]\n"); return 2; }
    double maxwait = argc > 2 ? atof(argv[2]) : 20;
    FILE *f = fopen(argv[1], "r");
    if (!f) return 1;
    size_t cap = 4096, n = 0;
    struct ent *e = malloc(cap * sizeof *e);
    char *line = NULL; size_t ll = 0;
    while (getline(&line, &ll, f) > 0) {
        char *t1 = strchr(line, '\t'); if (!t1) continue;
        char *t2 = strchr(t1 + 1, '\t'); if (!t2) continue;
        *t1 = 0;
        if (n == cap) { cap *= 2; e = realloc(e, cap * sizeof *e); }
        e[n].path = strdup(line); e[n].off = atoll(t1 + 1); e[n].len = atoll(t2 + 1); e[n].done = 0; n++;
    }
    fclose(f);
    double t0 = now();
    size_t left = n; long long bytes = 0;
    char *lastpath = NULL; int fd = -1;
    size_t stuck = (size_t)-1, skipped = 0; double stuckt = 0;
    while (left) {
        size_t progressed = 0;
        for (size_t i = 0; i < n; i++) {
            if (e[i].done) continue;
            if (!lastpath || strcmp(lastpath, e[i].path)) {
                if (fd >= 0) close(fd);
                fd = open(e[i].path, O_RDONLY | O_CLOEXEC);
                lastpath = e[i].path;
            }
            if (fd < 0) {                              // not there yet: keep order, wait and retry from here
                lastpath = NULL;
                if (stuck != i) { stuck = i; stuckt = now(); }
                else if (now() - stuckt > 4) {        // never shows up (e.g. user storage still locked): drop this file
                    char *p = e[i].path;
                    for (size_t j = i; j < n; j++) if (!e[j].done && !strcmp(e[j].path, p)) { e[j].done = 1; left--; skipped++; }
                    continue;
                }
                break;
            }
            // the kernel caps one readahead() call at its read-ahead window, so feed it in small pieces
            for (long long o = e[i].off, end = e[i].off + e[i].len; o < end; o += 131072)
                readahead(fd, o, end - o < 131072 ? end - o : 131072);
            bytes += e[i].len;
            e[i].done = 1; left--; progressed++;
        }
        if (!left) break;
        if (now() - t0 > maxwait) break;
        if (!progressed) usleep(50000);
    }
    if (fd >= 0) close(fd);
    FILE *k = fopen("/dev/kmsg", "w");
    if (k) { fprintf(k, "bootra: %zu/%zu ranges (%zu skipped), %lld MB requested, %.2fs\n", n - left - skipped, n, skipped, bytes >> 20, now() - t0); fclose(k); }
    return 0;
}
