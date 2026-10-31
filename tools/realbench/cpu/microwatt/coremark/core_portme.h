/* CoreMark port for Microwatt (realbench harness): the barebones port of EEMBC CoreMark (Apache-2.0, Copyright 2018 EEMBC, original
 * author Shay Gal-on) cut down to what this target needs. Integer only, static memory on the stack, time from the timebase register. */
#ifndef CORE_PORTME_H
#define CORE_PORTME_H

#define HAS_FLOAT  0
#define HAS_TIME_H 0
#define USE_CLOCK  0
#define HAS_STDIO  0
#define HAS_PRINTF 0

#ifndef COMPILER_VERSION
#define COMPILER_VERSION "realbench"
#endif
#ifndef COMPILER_FLAGS
#define COMPILER_FLAGS "see tools/realbench/programs.py"
#endif
#define MEM_LOCATION "STACK"

typedef signed short ee_s16;
typedef unsigned short ee_u16;
typedef signed int ee_s32;
typedef double ee_f32;
typedef unsigned char ee_u8;
typedef unsigned int ee_u32;
typedef unsigned long ee_ptr_int;
typedef unsigned long ee_size_t;
#define NULL ((void *)0)

#define align_mem(x) (void *)(4 + (((ee_ptr_int)(x)-1) & ~3))

#define CORETIMETYPE ee_u32
typedef ee_u32 CORE_TICKS;

#define SEED_METHOD SEED_VOLATILE
#define MEM_METHOD  MEM_STACK
#define MULTITHREAD 1
#define USE_PTHREAD 0
#define USE_FORK    0
#define USE_SOCKET  0
#define MAIN_HAS_NOARGC 1
#define MAIN_HAS_NORETURN 0

extern ee_u32 default_num_contexts;

typedef struct CORE_PORTABLE_S {
    ee_u8 portable_id;
} core_portable;

void portable_init(core_portable *p, int *argc, char *argv[]);
void portable_fini(core_portable *p);
int ee_printf(const char *fmt, ...);
void uart_send_char(char c);

#endif
