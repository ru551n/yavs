/* Exact user-instruction counter: single-steps a child with ptrace and counts how often each
 * instruction address executes, for a window of the run. Driven by stepcount.py (symbols, lines).
 *
 *   step [-f ADDR[:SKIP]] [-t ADDR[:N]] [-n MAX] -o PCFILE -- prog args...
 *
 * ADDR is a hex symbol address of the program file (as `nm` prints it); the load base is added.
 * -f: the window opens at the (SKIP+1)-th entry to ADDR (default: at the first instruction).
 * -t: the window closes at the N-th entry to ADDR after it opened (default 1).
 * -n: the window closes after MAX instructions. Without -t and -n it closes at exit.
 * -d ADDR -a ARMFILE [-s SYMFILE]: ADDR is the address of the indirect jump that dispatches the ops of an
 *    interpreter loop. Every instruction after a dispatch up to the next one (callees included) is charged
 *    to the op that dispatch started, named by its first instruction. ARMFILE gets "arm instructions ops"
 *    lines (hex arm, decimal), and with SYMFILE ("hex-address hex-size" lines, sorted) ARMFILE.fn gets
 *    "arm symbol-index instructions" lines: where the instructions of each op were spent.
 * -w ARM: also ARMFILE.pc gets "relative-pc count" lines for the instructions executed by the ops of that arm (hex).
 * PCFILE gets "relative-pc count" lines (hex pc, decimal) for addresses of the program file;
 * stderr gets the totals, and the instructions outside it (libraries, vdso) per mapping.
 * Only the main thread is traced: run the program with one thread. x86-64 Linux only.
 */
#define _GNU_SOURCE
#include <elf.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/personality.h>
#include <sys/ptrace.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

static void die(const char *m) {
    fprintf(stderr, "step: %s: %s\n", m, errno ? strerror(errno) : "failed");
    exit(2);
}

/* Open-addressing table pc -> count. */
static struct { uint64_t pc, n; } *tab;
static uint64_t cap = 1 << 20, used;

static void bump(uint64_t pc);
static uint64_t watch_arm;
static struct { uint64_t pc, n; } *wtab;
static uint64_t wcap = 1 << 16, wused;

/* The instructions of one watched arm, by pc (table doubles on demand). */
static void wbump(uint64_t pc) {
    uint64_t i = (pc * 0x9E3779B97F4A7C15ull) >> 40 & (wcap - 1);
    for (;; i = (i + 1) & (wcap - 1)) {
        if (wtab[i].n && wtab[i].pc == pc) { wtab[i].n++; return; }
        if (!wtab[i].n) break;
    }
    if (wused * 2 >= wcap) {
        void *old = wtab; uint64_t oc = wcap;
        wcap *= 2; wused = 0;
        wtab = calloc(wcap, sizeof *wtab);
        if (!wtab) die("calloc");
        for (uint64_t k = 0; k < oc; k++) {
            typeof(*wtab) *e = (typeof(*wtab) *)old + k;
            if (!e->n) continue;
            uint64_t j = (e->pc * 0x9E3779B97F4A7C15ull) >> 40 & (wcap - 1);
            while (wtab[j].n) j = (j + 1) & (wcap - 1);
            wtab[j] = *e; wused++;
        }
        free(old);
        wbump(pc); return;
    }
    wtab[i].pc = pc; wtab[i].n = 1; wused++;
}

static void bump(uint64_t pc) {
    uint64_t i = (pc * 0x9E3779B97F4A7C15ull) >> 40 & (cap - 1);
    for (;; i = (i + 1) & (cap - 1)) {
        if (tab[i].n && tab[i].pc == pc) { tab[i].n++; return; }
        if (!tab[i].n) break;
    }
    if (used * 2 >= cap) { /* grow */
        void *old = tab; uint64_t oc = cap;
        cap *= 2; used = 0;
        tab = calloc(cap, sizeof *tab);
        if (!tab) die("calloc");
        for (uint64_t k = 0; k < oc; k++) {
            typeof(*tab) *e = (typeof(*tab) *)old + k;
            if (!e->n) continue;
            uint64_t j = (e->pc * 0x9E3779B97F4A7C15ull) >> 40 & (cap - 1);
            while (tab[j].n) j = (j + 1) & (cap - 1);
            tab[j] = *e; used++;
        }
        free(old);
        bump(pc); return;
    }
    tab[i].pc = pc; tab[i].n = 1; used++;
}

/* Instructions per dispatched op ("arm": the first instruction after the dispatch jump), and per
 * (arm, symbol). */
static struct { uint64_t arm, ins, ops; } arms[4096];
static int narms;
static uint64_t *symaddr, *symsize;
static int nsym;
static struct { uint64_t arm; int fn; uint64_t n; } *af;
static int naf, capaf;

static int arm_index(uint64_t arm) {
    for (int i = 0; i < narms; i++) if (arms[i].arm == arm) return i;
    if (narms >= 4096) return 0;
    arms[narms].arm = arm;
    return narms++;
}

static int symbol_of(uint64_t pc) {
    int lo = 0, hi = nsym - 1, r = -1;
    while (lo <= hi) {
        int m = (lo + hi) / 2;
        if (symaddr[m] <= pc) { r = m; lo = m + 1; } else hi = m - 1;
    }
    return r >= 0 && pc < symaddr[r] + symsize[r] ? r : -1;
}

static void arm_fn_bump(uint64_t arm, int fn) {
    for (int i = 0; i < naf; i++) if (af[i].arm == arm && af[i].fn == fn) { af[i].n++; return; }
    if (naf == capaf) {
        capaf = capaf ? capaf * 2 : 4096;
        af = realloc(af, capaf * sizeof *af);
        if (!af) die("realloc");
    }
    af[naf].arm = arm; af[naf].fn = fn; af[naf].n = 1; naf++;
}

/* Load base of the program file and its mappings (lowest start, end) from /proc/PID/maps. */
static char exe[4096];
static struct { uint64_t lo, hi; char name[256]; uint64_t n; int is_exe; } maps[256];
static int nmaps;

static uint64_t load_maps(pid_t pid, uint64_t first_vaddr) {
    char p[64], line[4600];
    snprintf(p, sizeof p, "/proc/%d/maps", pid);
    FILE *f = fopen(p, "r");
    if (!f) die("open maps");
    uint64_t base = 0; int have = 0; nmaps = 0;
    while (fgets(line, sizeof line, f) && nmaps < 256) {
        uint64_t lo, hi, off; char name[4096] = "";
        if (sscanf(line, "%" SCNx64 "-%" SCNx64 " %*s %" SCNx64 " %*s %*s %4095[^\n]", &lo, &hi, &off, name) < 3) continue;
        char *nm = name; while (*nm == ' ') nm++;
        if (!have && off == 0 && !strcmp(nm, exe)) { base = lo - first_vaddr; have = 1; }
        maps[nmaps].lo = lo; maps[nmaps].hi = hi;
        snprintf(maps[nmaps].name, sizeof maps[nmaps].name, "%s", *nm ? nm : "[anon]");
        maps[nmaps].is_exe = !strcmp(nm, exe);
        nmaps++;
    }
    fclose(f);
    if (!have) { errno = 0; die("program mapping not found"); }
    return base;
}

static uint64_t first_load(const char *path, int *pie) {
    int fd = open(path, O_RDONLY);
    Elf64_Ehdr h; Elf64_Phdr ph;
    if (fd < 0 || pread(fd, &h, sizeof h, 0) != sizeof h) die("read ELF header");
    *pie = h.e_type == ET_DYN;
    for (int i = 0; i < h.e_phnum; i++) {
        if (pread(fd, &ph, sizeof ph, h.e_phoff + (off_t)i * h.e_phentsize) != sizeof ph) die("read ELF phdr");
        if (ph.p_type == PT_LOAD && ph.p_offset == 0) { close(fd); return ph.p_vaddr; }
    }
    errno = 0; die("no PT_LOAD at offset 0");
    return 0;
}

static int cmp_pc(const void *a, const void *b) {
    uint64_t x = *(uint64_t *)a, y = *(uint64_t *)b;
    return x < y ? -1 : x > y;
}

int main(int argc, char **argv) {
    uint64_t from = 0, skip = 0, to = 0, nto = 1, max = 0, disp = 0;
    const char *out = NULL, *armf = NULL, *symf = NULL;
    int c;
    while ((c = getopt(argc, argv, "+f:t:n:o:d:a:s:w:")) != -1) {
        char *e;
        switch (c) {
        case 'f': from = strtoull(optarg, &e, 16); if (*e == ':') skip = strtoull(e + 1, NULL, 10); break;
        case 't': to = strtoull(optarg, &e, 16); if (*e == ':') nto = strtoull(e + 1, NULL, 10); break;
        case 'n': max = strtoull(optarg, NULL, 10); break;
        case 'o': out = optarg; break;
        case 'd': disp = strtoull(optarg, NULL, 16); break;
        case 'a': armf = optarg; break;
        case 's': symf = optarg; break;
        case 'w': watch_arm = strtoull(optarg, NULL, 16); break;
        default: return 2;
        }
    }
    if (!out || optind >= argc) { fprintf(stderr, "usage: step [-f ADDR[:SKIP]] [-t ADDR[:N]] [-n MAX] -o PCFILE -- prog args...\n"); return 2; }
    if (disp && !armf) { fprintf(stderr, "step: -d needs -a\n"); return 2; }
    if (symf) {
        FILE *sf = fopen(symf, "r");
        uint64_t a, z;
        int cap = 1 << 18;
        if (!sf) die(symf);
        symaddr = malloc(cap * sizeof *symaddr);
        symsize = malloc(cap * sizeof *symsize);
        if (!symaddr || !symsize) die("malloc");
        while (fscanf(sf, "%" SCNx64 " %" SCNx64, &a, &z) == 2 && nsym < cap) { symaddr[nsym] = a; symsize[nsym++] = z; }
        fclose(sf);
    }
    char **prog = argv + optind;
    if (!realpath(prog[0], exe)) die(prog[0]);
    int pie; uint64_t fv = first_load(exe, &pie);
    tab = calloc(cap, sizeof *tab);
    if (!tab) die("calloc");
    wtab = calloc(wcap, sizeof *wtab);
    if (!wtab) die("calloc");

    pid_t pid = fork();
    if (pid < 0) die("fork");
    if (pid == 0) {
        personality(ADDR_NO_RANDOMIZE);
        if (ptrace(PTRACE_TRACEME, 0, 0, 0) < 0) die("TRACEME");
        execv(exe, prog);
        die("exec");
    }
    int st;
    if (waitpid(pid, &st, 0) < 0 || !WIFSTOPPED(st)) { errno = 0; die("child did not stop at exec"); }
    uint64_t base = load_maps(pid, fv);
    if (!pie) base = 0;

    struct user_regs_struct r;
    int open_win = from == 0;
    if (!open_win) { /* run to the (skip+1)-th entry to `from` with a breakpoint */
        uint64_t a = base + from;
        long w = ptrace(PTRACE_PEEKTEXT, pid, a, 0);
        if (w == -1 && errno) die("peek (is --from an address of the program?)");
        long bp = (w & ~0xffL) | 0xcc;
        uint64_t hits = 0;
        for (;;) {
            if (ptrace(PTRACE_POKETEXT, pid, a, bp) < 0) die("poke");
            if (ptrace(PTRACE_CONT, pid, 0, 0) < 0) die("cont");
            if (waitpid(pid, &st, 0) < 0) die("wait");
            if (WIFEXITED(st) || WIFSIGNALED(st)) { errno = 0; die("the program ended before the window opened"); }
            ptrace(PTRACE_POKETEXT, pid, a, w);
            ptrace(PTRACE_GETREGS, pid, 0, &r);
            if (WSTOPSIG(st) != SIGTRAP || r.rip != a + 1) { errno = 0; fprintf(stderr, "step: stopped by signal %d\n", WSTOPSIG(st)); return 2; }
            r.rip = a; ptrace(PTRACE_SETREGS, pid, 0, &r);
            if (hits++ == skip) break;
            ptrace(PTRACE_SINGLESTEP, pid, 0, 0); /* over the original instruction */
            waitpid(pid, &st, 0);
        }
        open_win = 1;
    }
    load_maps(pid, fv); /* mappings of the libraries exist now */

    uint64_t total = 0, outside = 0, tos = 0, cur = 0;
    int after_dispatch = 0;
    int ended = 0;
    for (;;) {
        if (ptrace(PTRACE_GETREGS, pid, 0, &r) < 0) die("getregs");
        uint64_t pc = r.rip;
        if (to && pc == base + to && ++tos == nto) break;
        if (max && total >= max) break;
        total++;
        uint64_t rel = pc - base;
        int in = 0;
        for (int i = 0; i < nmaps; i++)
            if (pc >= maps[i].lo && pc < maps[i].hi) { if (maps[i].is_exe) in = 1; else maps[i].n++; break; }
        if (in) bump(rel); else outside++;
        if (disp) {
            if (after_dispatch) { cur = rel; after_dispatch = 0; arms[arm_index(cur)].ops++; }
            if (cur) {
                arms[arm_index(cur)].ins++;
                if (watch_arm && cur == watch_arm && in) wbump(rel);
                if (nsym) arm_fn_bump(cur, in ? symbol_of(rel) : -1);
            }
            if (in && rel == disp) after_dispatch = 1;
        }
        if (ptrace(PTRACE_SINGLESTEP, pid, 0, 0) < 0) die("step");
        if (waitpid(pid, &st, 0) < 0) die("wait");
        if (WIFEXITED(st) || WIFSIGNALED(st)) { ended = 1; break; }
        if (WSTOPSIG(st) != SIGTRAP) { /* deliver a signal the program should see */
            if (ptrace(PTRACE_SINGLESTEP, pid, 0, WSTOPSIG(st)) < 0) die("signal");
            waitpid(pid, &st, 0);
            if (WIFEXITED(st) || WIFSIGNALED(st)) { ended = 1; break; }
        }
    }
    if (!ended) { ptrace(PTRACE_KILL, pid, 0, 0); kill(pid, 9); waitpid(pid, &st, 0); }

    FILE *f = fopen(out, "w");
    if (!f) die(out);
    uint64_t (*v)[2] = malloc(used * sizeof *v); uint64_t k = 0;
    for (uint64_t i = 0; i < cap; i++) if (tab[i].n) { v[k][0] = tab[i].pc; v[k][1] = tab[i].n; k++; }
    qsort(v, k, sizeof *v, cmp_pc);
    for (uint64_t i = 0; i < k; i++) fprintf(f, "%" PRIx64 " %" PRIu64 "\n", v[i][0], v[i][1]);
    fclose(f);
    if (disp) {
        char p[4200];
        FILE *g = fopen(armf, "w");
        if (!g) die(armf);
        for (int i = 0; i < narms; i++) fprintf(g, "%" PRIx64 " %" PRIu64 " %" PRIu64 "\n", arms[i].arm, arms[i].ins, arms[i].ops);
        fclose(g);
        if (watch_arm) {
            snprintf(p, sizeof p, "%s.pc", armf);
            g = fopen(p, "w");
            if (!g) die(p);
            for (uint64_t i = 0; i < wcap; i++) if (wtab[i].n) fprintf(g, "%" PRIx64 " %" PRIu64 "\n", wtab[i].pc, wtab[i].n);
            fclose(g);
        }
        if (nsym) {
            snprintf(p, sizeof p, "%s.fn", armf);
            g = fopen(p, "w");
            if (!g) die(p);
            for (int i = 0; i < naf; i++) fprintf(g, "%" PRIx64 " %d %" PRIu64 "\n", af[i].arm, af[i].fn, af[i].n);
            fclose(g);
        }
    }
    fprintf(stderr, "step: %" PRIu64 " instructions in the window (%" PRIu64 " outside the program file)%s\n",
            total, outside, ended ? ", the program ended" : "");
    for (int i = 0; i < nmaps; i++) if (maps[i].n) fprintf(stderr, "step:   outside: %s %" PRIu64 "\n", maps[i].name, maps[i].n);
    return 0;
}
