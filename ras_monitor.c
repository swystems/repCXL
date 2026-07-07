/**
 * @brief Watch a running repCXL process and, if it disappears, correlate the
 *        exit with hardware memory errors recorded by rasdaemon (poisoning,
 *        ECC errors, CXL general-media/DRAM/module events, hot-unplug, etc.)
 *        against the memory nodes listed in a repCXL config TOML file.
 *
 * Usage:
 *   ras_monitor -p PID -c config/local.toml [-d RASDB] [-v]
 *
 * See RAS_MONITOR.md for a full description, build instructions, and the
 * limitations of the heuristics used below.
 *
 * Manual build (if not using `make`):
 *   gcc -O2 -Wall -Wextra -std=gnu11 -o ras_monitor ras_monitor.c \
 *       $(pkg-config --cflags --libs sqlite3)
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>
#include <errno.h>
#include <ctype.h>
#include <limits.h>
#include <signal.h>
#include <time.h>
#include <poll.h>
#include <sys/types.h>
#include <sys/syscall.h>
#include <sqlite3.h>

#ifndef SYS_pidfd_open
#define SYS_pidfd_open 434
#endif

#define MAX_NODES   64
#define MAX_IDENTS  32
#define MAX_ADDRS   16
#define MAX_MATCHES 128

/* ----------------------------------------------------------------------
 * Configured memory node identity, resolved from config TOML + sysfs
 * ---------------------------------------------------------------------- */

typedef struct {
    int  index;                 /* array position in mem_nodes, -1 for logger_node */
    char label[32];              /* "node 0", "logger" ... */
    char path[PATH_MAX];         /* raw path from the config file */
    char dax_name[64];           /* "dax0.1" if path is a /dev/dax device */
    char memdev[64];             /* best-effort resolved CXL "memN" device name */
    char pci_bdf[32];             /* PCI BDF if path is a /sys/bus/pci/... resource */
    int  numa_node;               /* -1 if unknown */
    unsigned long long spa_start; /* system physical address range, if known */
    unsigned long long spa_end;
    int  has_spa;
} node_id_t;

static node_id_t g_nodes[MAX_NODES];
static int g_num_nodes = 0;

typedef struct {
    int node_index;
    const char *table;
    const char *confidence;
    char evidence[512];
} match_t;

static match_t g_matches[MAX_MATCHES];
static int g_num_matches = 0;

/* ----------------------------------------------------------------------
 * Small utilities
 * ---------------------------------------------------------------------- */

static int read_sysfs_line(const char *path, char *buf, size_t bufsz) {
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    if (!fgets(buf, bufsz, f)) { fclose(f); return -1; }
    fclose(f);
    size_t n = strlen(buf);
    while (n && (buf[n - 1] == '\n' || buf[n - 1] == '\r')) buf[--n] = 0;
    return 0;
}

static int read_sysfs_u64(const char *path, unsigned long long *out) {
    char buf[64];
    if (read_sysfs_line(path, buf, sizeof(buf)) != 0) return -1;
    char *end;
    errno = 0;
    unsigned long long v = strtoull(buf, &end, 0);
    if (end == buf) return -1;
    *out = v;
    return 0;
}

static int read_sysfs_long(const char *path, long *out) {
    char buf[64];
    if (read_sysfs_line(path, buf, sizeof(buf)) != 0) return -1;
    *out = strtol(buf, NULL, 0);
    return 0;
}

static int pidfd_open_wrap(pid_t pid, unsigned int flags) {
    return (int)syscall(SYS_pidfd_open, pid, flags);
}

static int process_alive(pid_t pid) {
    if (kill(pid, 0) == 0) return 1;
    return errno != ESRCH;
}

/* ----------------------------------------------------------------------
 * Node identity resolution (config path -> sysfs identity)
 * ---------------------------------------------------------------------- */

/* matches ^mem[0-9]+$ - the CXL "memN" device name, not to be confused with
 * the unrelated /sys/devices/system/memory/memoryN hotplug blocks. */
static int looks_like_memdev(const char *comp, char *out, size_t outsz) {
    size_t len = strlen(comp);
    if (len < 4 || comp[0] != 'm' || comp[1] != 'e' || comp[2] != 'm') return 0;
    for (size_t i = 3; i < len; i++)
        if (!isdigit((unsigned char)comp[i])) return 0;
    snprintf(out, outsz, "%s", comp);
    return 1;
}

static void resolve_memdev_from_realpath(const char *sysfs_dir, char *memdev, size_t memdev_sz) {
    char real[PATH_MAX];
    memdev[0] = 0;
    if (!realpath(sysfs_dir, real)) return;

    char *copy = strdup(real);
    if (!copy) return;

    char *saveptr = NULL;
    char *tok = strtok_r(copy, "/", &saveptr);
    while (tok) {
        char candidate[64];
        if (looks_like_memdev(tok, candidate, sizeof(candidate)))
            snprintf(memdev, memdev_sz, "%s", candidate);
        tok = strtok_r(NULL, "/", &saveptr);
    }
    free(copy);
}

static void resolve_node(node_id_t *n) {
    n->numa_node = -1;
    n->has_spa = 0;
    n->dax_name[0] = 0;
    n->memdev[0] = 0;
    n->pci_bdf[0] = 0;

    if (strncmp(n->path, "/dev/dax", 8) == 0) {
        const char *base = strrchr(n->path, '/');
        base = base ? base + 1 : n->path;
        snprintf(n->dax_name, sizeof(n->dax_name), "%s", base);

        char sysfs_dir[PATH_MAX];
        snprintf(sysfs_dir, sizeof(sysfs_dir), "/sys/bus/dax/devices/%s", n->dax_name);

        char p[PATH_MAX + 32];
        long numa;
        snprintf(p, sizeof(p), "%s/target_node", sysfs_dir);
        if (read_sysfs_long(p, &numa) == 0) n->numa_node = (int)numa;

        unsigned long long start = 0, end = 0, size = 0;
        snprintf(p, sizeof(p), "%s/mapping0/start", sysfs_dir);
        int have_start = (read_sysfs_u64(p, &start) == 0);
        snprintf(p, sizeof(p), "%s/mapping0/end", sysfs_dir);
        int have_end = (read_sysfs_u64(p, &end) == 0);
        if (have_start && !have_end) {
            snprintf(p, sizeof(p), "%s/mapping0/size", sysfs_dir);
            if (read_sysfs_u64(p, &size) == 0) { end = start + size; have_end = 1; }
        }
        if (have_start && have_end) {
            n->spa_start = start;
            n->spa_end = end;
            n->has_spa = 1;
        }

        resolve_memdev_from_realpath(sysfs_dir, n->memdev, sizeof(n->memdev));
    } else if (strncmp(n->path, "/sys/bus/pci/devices/", 21) == 0) {
        const char *rest = n->path + 21;
        const char *slash = strchr(rest, '/');
        size_t len = slash ? (size_t)(slash - rest) : strlen(rest);
        if (len >= sizeof(n->pci_bdf)) len = sizeof(n->pci_bdf) - 1;
        memcpy(n->pci_bdf, rest, len);
        n->pci_bdf[len] = 0;
    }
    /* else: plain file (e.g. /dev/shm/...) - no hardware identity available */
}

/* ----------------------------------------------------------------------
 * Minimal config TOML scraper - only needs mem_nodes[] and logger_node
 * ---------------------------------------------------------------------- */

static void strip_comment(char *line) {
    char *h = strchr(line, '#');
    if (h) *h = 0;
}

static void extract_quoted_paths(const char *s, int *next_index) {
    const char *p = s;
    while ((p = strchr(p, '"')) != NULL) {
        p++;
        const char *end = strchr(p, '"');
        if (!end) break;
        if (g_num_nodes < MAX_NODES) {
            node_id_t *n = &g_nodes[g_num_nodes];
            size_t len = (size_t)(end - p);
            if (len >= sizeof(n->path)) len = sizeof(n->path) - 1;
            memcpy(n->path, p, len);
            n->path[len] = 0;
            n->index = (*next_index)++;
            snprintf(n->label, sizeof(n->label), "node %d", n->index);
            g_num_nodes++;
        }
        p = end + 1;
    }
}

static int load_config(const char *path) {
    FILE *f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "error: cannot open config '%s': %s\n", path, strerror(errno));
        return -1;
    }

    char line[1024];
    int in_array = 0;
    int next_index = 0;
    char logger_path[PATH_MAX];
    logger_path[0] = 0;

    while (fgets(line, sizeof(line), f)) {
        strip_comment(line);
        if (!in_array) {
            if (strstr(line, "mem_nodes")) {
                char *br = strchr(line, '[');
                if (!br) continue;
                in_array = 1;
                char *close = strchr(br, ']');
                if (close) { *close = 0; in_array = 0; }
                extract_quoted_paths(br, &next_index);
                continue;
            }
            if (strstr(line, "logger_node")) {
                char *p = strchr(line, '"');
                if (p) {
                    p++;
                    char *end = strchr(p, '"');
                    if (end) {
                        size_t len = (size_t)(end - p);
                        if (len >= sizeof(logger_path)) len = sizeof(logger_path) - 1;
                        memcpy(logger_path, p, len);
                        logger_path[len] = 0;
                    }
                }
                continue;
            }
        } else {
            char *close = strchr(line, ']');
            if (close) { *close = 0; in_array = 0; }
            extract_quoted_paths(line, &next_index);
        }
    }
    fclose(f);

    if (logger_path[0] && g_num_nodes < MAX_NODES) {
        node_id_t *n = &g_nodes[g_num_nodes++];
        n->index = -1;
        snprintf(n->label, sizeof(n->label), "logger");
        snprintf(n->path, sizeof(n->path), "%s", logger_path);
    }

    for (int i = 0; i < g_num_nodes; i++) resolve_node(&g_nodes[i]);

    return g_num_nodes > 0 ? 0 : -1;
}

/* ----------------------------------------------------------------------
 * rasdaemon sqlite3 db scanning
 * ---------------------------------------------------------------------- */

/* Every table rasdaemon writes may or may not exist depending on the
 * kernel/rasdaemon version and which tracepoints were compiled in, so we
 * probe for each one instead of assuming a fixed schema. */
static const char *RAS_TABLES[] = {
    "mc_event",
    "aer_event",
    "mce_record",
    "extlog_record",
    "arm_event",
    "non_standard_record",
    "devlink_event",
    "diskerror_event",
    "memory_failure_event",
    "cxl_poison_event",
    "cxl_aer_ue_event",
    "cxl_aer_ce_event",
    "cxl_overflow_event",
    "cxl_general_media_event",
    "cxl_dram_event",
    "cxl_memory_module_event",
    "cxl_dcd_event",
    NULL
};

static int table_exists(sqlite3 *db, const char *name) {
    sqlite3_stmt *st;
    const char *q = "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?";
    if (sqlite3_prepare_v2(db, q, -1, &st, NULL) != SQLITE_OK) return 0;
    sqlite3_bind_text(st, 1, name, -1, SQLITE_STATIC);
    int found = (sqlite3_step(st) == SQLITE_ROW);
    sqlite3_finalize(st);
    return found;
}

static int has_column(sqlite3 *db, const char *table, const char *colname) {
    char q[256];
    snprintf(q, sizeof(q), "PRAGMA table_info(%s)", table);
    sqlite3_stmt *st;
    int found = 0;
    if (sqlite3_prepare_v2(db, q, -1, &st, NULL) != SQLITE_OK) return 0;
    while (sqlite3_step(st) == SQLITE_ROW) {
        const char *name = (const char *)sqlite3_column_text(st, 1);
        if (name && strcasecmp(name, colname) == 0) { found = 1; break; }
    }
    sqlite3_finalize(st);
    return found;
}

static int is_identity_col(const char *name) {
    static const char *keys[] = { "memdev", "serial", "label", "dimm", "location", "dev_name", "host", "region", NULL };
    for (int i = 0; keys[i]; i++)
        if (strcasestr(name, keys[i])) return 1;
    return 0;
}

static int is_address_col(const char *name) {
    static const char *keys[] = { "dpa", "hpa", "addr", "pfn", NULL };
    for (int i = 0; keys[i]; i++)
        if (strcasestr(name, keys[i])) return 1;
    return 0;
}

static void record_match(int node_index, const char *table, const char *confidence, const char *fmt, ...) {
    if (g_num_matches >= MAX_MATCHES) return;
    match_t *m = &g_matches[g_num_matches++];
    m->node_index = node_index;
    m->table = table;
    m->confidence = confidence;
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(m->evidence, sizeof(m->evidence), fmt, ap);
    va_end(ap);
}

static void match_row_against_nodes(const char *table, const char *timestamp,
                                     char idents[][256], int n_idents,
                                     unsigned long long *addrs, int n_addrs) {
    for (int i = 0; i < g_num_nodes; i++) {
        node_id_t *n = &g_nodes[i];

        for (int j = 0; j < n_idents; j++) {
            if (n->dax_name[0] && strcasestr(idents[j], n->dax_name))
                record_match(n->index, table, "high",
                             "%s @ %s: identity match on \"%s\" (dax_name=%s)",
                             table, timestamp, idents[j], n->dax_name);
            if (n->memdev[0] && strcasestr(idents[j], n->memdev))
                record_match(n->index, table, "high",
                             "%s @ %s: identity match on \"%s\" (memdev=%s)",
                             table, timestamp, idents[j], n->memdev);
            if (n->pci_bdf[0] && strcasestr(idents[j], n->pci_bdf))
                record_match(n->index, table, "medium",
                             "%s @ %s: identity match on \"%s\" (pci_bdf=%s)",
                             table, timestamp, idents[j], n->pci_bdf);
        }

        if (n->has_spa) {
            for (int j = 0; j < n_addrs; j++) {
                if (addrs[j] >= n->spa_start && addrs[j] < n->spa_end)
                    record_match(n->index, table, "medium",
                                 "%s @ %s: address 0x%llx within node SPA range [0x%llx-0x%llx)",
                                 table, timestamp, addrs[j], n->spa_start, n->spa_end);
            }
        }
    }
}

static int scan_table(sqlite3 *db, const char *table, const char *since, int verbose) {
    int has_ts = has_column(db, table, "timestamp");
    char q[512];
    if (has_ts)
        snprintf(q, sizeof(q), "SELECT * FROM %s WHERE timestamp >= ? ORDER BY timestamp ASC", table);
    else
        snprintf(q, sizeof(q), "SELECT * FROM %s", table);

    sqlite3_stmt *st;
    if (sqlite3_prepare_v2(db, q, -1, &st, NULL) != SQLITE_OK) {
        fprintf(stderr, "warning: failed to query %s: %s\n", table, sqlite3_errmsg(db));
        return 0;
    }
    if (has_ts) sqlite3_bind_text(st, 1, since, -1, SQLITE_STATIC);

    int rows = 0;
    int ncols = sqlite3_column_count(st);

    while (sqlite3_step(st) == SQLITE_ROW) {
        rows++;
        char idents[MAX_IDENTS][256];
        int n_idents = 0;
        unsigned long long addrs[MAX_ADDRS];
        int n_addrs = 0;
        char dump[2048];
        dump[0] = 0;
        char timestamp[64] = "?";

        for (int c = 0; c < ncols; c++) {
            const char *cname = sqlite3_column_name(st, c);
            const unsigned char *val = sqlite3_column_text(st, c);
            const char *v = val ? (const char *)val : "";

            if (strcasecmp(cname, "timestamp") == 0)
                snprintf(timestamp, sizeof(timestamp), "%s", v);

            char piece[300];
            snprintf(piece, sizeof(piece), "%s=%s ", cname, v);
            strncat(dump, piece, sizeof(dump) - strlen(dump) - 1);

            if (val && v[0] && is_identity_col(cname) && n_idents < MAX_IDENTS)
                snprintf(idents[n_idents++], 256, "%s", v);

            if (val && v[0] && is_address_col(cname) && n_addrs < MAX_ADDRS) {
                unsigned long long a = strtoull(v, NULL, 0);
                if (a) addrs[n_addrs++] = a;
            }
        }

        if (verbose) printf("  [%s] %s\n", table, dump);

        match_row_against_nodes(table, timestamp, idents, n_idents, addrs, n_addrs);
    }
    sqlite3_finalize(st);
    return rows;
}

/* ----------------------------------------------------------------------
 * main
 * ---------------------------------------------------------------------- */

static void usage(const char *prog) {
    fprintf(stderr,
        "usage: %s -p PID -c CONFIG.toml [options]\n"
        "\n"
        "options:\n"
        "  -p PID        pid of the running repCXL process to watch (required unless -s is given)\n"
        "  -c CONFIG     path to the repCXL TOML config (defines mem_nodes / logger_node) (required)\n"
        "  -d PATH       rasdaemon sqlite3 db (default: /var/lib/rasdaemon/ras-mc_event.db)\n"
        "  -i MS         poll interval in ms for the fallback (non-pidfd) watch path (default: 500)\n"
        "  -g SEC        grace period after exit before querying the db (default: 2)\n"
        "  -t SEC        give up waiting for exit after SEC seconds, 0 = wait forever (default: 0)\n"
        "  -s \"TIME\"     skip process monitoring; query the db for events since TIME\n"
        "                (format: \"YYYY-MM-DD HH:MM:SS\") - use for post-hoc analysis\n"
        "  -v            verbose: print every RAS row found, not just correlated matches\n"
        "  -h            show this help\n"
        "\n"
        "exit codes:\n"
        "  0  process exited and a RAS event was correlated to a configured memory node\n"
        "  1  process exited, but no relevant RAS events were found in the window\n"
        "  2  process exited and RAS events were found, but none matched a configured node\n"
        "  3  usage / setup error\n",
        prog);
}

int main(int argc, char **argv) {
    pid_t pid = -1;
    const char *config_path = NULL;
    const char *db_path = "/var/lib/rasdaemon/ras-mc_event.db";
    int poll_ms = 500;
    int grace_sec = 2;
    int timeout_sec = 0;
    const char *since_override = NULL;
    int verbose = 0;

    int opt;
    while ((opt = getopt(argc, argv, "p:c:d:i:g:t:s:vh")) != -1) {
        switch (opt) {
        case 'p': pid = (pid_t)atoi(optarg); break;
        case 'c': config_path = optarg; break;
        case 'd': db_path = optarg; break;
        case 'i': poll_ms = atoi(optarg); break;
        case 'g': grace_sec = atoi(optarg); break;
        case 't': timeout_sec = atoi(optarg); break;
        case 's': since_override = optarg; break;
        case 'v': verbose = 1; break;
        case 'h': usage(argv[0]); return 0;
        default:  usage(argv[0]); return 3;
        }
    }

    if (!config_path || (pid <= 0 && !since_override)) {
        usage(argv[0]);
        return 3;
    }

    if (load_config(config_path) != 0) {
        fprintf(stderr, "error: failed to load any mem_nodes from '%s'\n", config_path);
        return 3;
    }

    printf("resolved %d configured memory node(s) from %s:\n", g_num_nodes, config_path);
    for (int i = 0; i < g_num_nodes; i++) {
        node_id_t *n = &g_nodes[i];
        printf("  %-8s %-32s", n->label, n->path);
        if (n->dax_name[0]) printf(" dax=%s", n->dax_name);
        if (n->numa_node >= 0) printf(" numa=%d", n->numa_node);
        if (n->has_spa) printf(" spa=[0x%llx-0x%llx)", n->spa_start, n->spa_end);
        if (n->memdev[0]) printf(" memdev=%s", n->memdev);
        if (n->pci_bdf[0]) printf(" pci=%s", n->pci_bdf);
        if (!n->dax_name[0] && !n->pci_bdf[0])
            printf(" (no hardware identity resolved - RAS correlation limited)");
        printf("\n");
    }
    printf("\n");

    char since_buf[64];

    if (since_override) {
        snprintf(since_buf, sizeof(since_buf), "%s", since_override);
    } else {
        if (kill(pid, 0) != 0) {
            fprintf(stderr, "error: pid %d not found (%s)\n", (int)pid, strerror(errno));
            return 3;
        }

        char comm_path[64], comm[256] = "?";
        snprintf(comm_path, sizeof(comm_path), "/proc/%d/comm", (int)pid);
        read_sysfs_line(comm_path, comm, sizeof(comm));

        time_t start = time(NULL);
        struct tm tmv;
        localtime_r(&start, &tmv);
        strftime(since_buf, sizeof(since_buf), "%Y-%m-%d %H:%M:%S %z", &tmv);

        printf("monitoring pid %d (%s); waiting for it to exit...\n", (int)pid, comm);

        time_t deadline = timeout_sec > 0 ? start + timeout_sec : 0;
        int pfd = pidfd_open_wrap(pid, 0);
        int timed_out = 0;

        if (pfd >= 0) {
            struct pollfd pfds = { .fd = pfd, .events = POLLIN, .revents = 0 };
            for (;;) {
                if (deadline && time(NULL) >= deadline) { timed_out = 1; break; }
                int ret = poll(&pfds, 1, poll_ms);
                if (ret > 0) break;
                if (!process_alive(pid)) break;
            }
            close(pfd);
        } else {
            while (process_alive(pid)) {
                if (deadline && time(NULL) >= deadline) { timed_out = 1; break; }
                struct timespec ts = { poll_ms / 1000, (long)(poll_ms % 1000) * 1000000L };
                nanosleep(&ts, NULL);
            }
        }

        if (timed_out) {
            fprintf(stderr, "timed out after %ds waiting for pid %d to exit\n", timeout_sec, (int)pid);
            return 3;
        }

        time_t detected = time(NULL);
        char detected_buf[64];
        struct tm dtm;
        localtime_r(&detected, &dtm);
        strftime(detected_buf, sizeof(detected_buf), "%Y-%m-%d %H:%M:%S %z", &dtm);
        printf("pid %d is no longer running (detected at %s)\n", (int)pid, detected_buf);

        if (grace_sec > 0) {
            printf("waiting %ds for rasdaemon to flush pending events...\n", grace_sec);
            sleep((unsigned int)grace_sec);
        }
    }

    printf("\nquerying rasdaemon db '%s' for events since %s\n\n", db_path, since_buf);

    sqlite3 *db;
    if (sqlite3_open_v2(db_path, &db, SQLITE_OPEN_READONLY, NULL) != SQLITE_OK) {
        fprintf(stderr, "error: cannot open rasdaemon db '%s': %s\n", db_path, sqlite3_errmsg(db));
        fprintf(stderr, "hint: is rasdaemon installed and running with the sqlite3 backend enabled?\n");
        return 3;
    }

    int total_rows = 0;
    for (int i = 0; RAS_TABLES[i]; i++) {
        if (!table_exists(db, RAS_TABLES[i])) continue;
        total_rows += scan_table(db, RAS_TABLES[i], since_buf, verbose);
    }
    sqlite3_close(db);

    if (total_rows == 0) {
        printf("no memory-related RAS events recorded during the monitoring window.\n");
        printf("the process exit may be unrelated to memory errors, or rasdaemon is not capturing the relevant tracepoints.\n");
        return 1;
    }

    if (g_num_matches == 0) {
        printf("%d RAS event(s) found in the window, but none correlate to a configured memory node.\n", total_rows);
        return 2;
    }

    printf("================================================================\n");
    for (int i = 0; i < g_num_matches; i++) {
        match_t *m = &g_matches[i];
        const char *label = "?";
        const char *path = "?";
        for (int j = 0; j < g_num_nodes; j++) {
            if (g_nodes[j].index == m->node_index) { label = g_nodes[j].label; path = g_nodes[j].path; break; }
        }
        printf("FAILED MEMORY NODE: %s (%s)  [confidence: %s]\n  %s\n", label, path, m->confidence, m->evidence);
    }
    printf("================================================================\n");

    return 0;
}
