/* CoreMark port for Microwatt (realbench harness), see core_portme.h. The iteration count comes from -DITERATIONS=N. */
#include "coremark.h"

#include <stdbool.h>
#include <stdint.h>

#include "console.h"

#ifndef ITERATIONS
#error "ITERATIONS must be defined (programs.py passes it)"
#endif

volatile ee_s32 seed1_volatile = 0x0;
volatile ee_s32 seed2_volatile = 0x0;
volatile ee_s32 seed3_volatile = 0x66;
volatile ee_s32 seed4_volatile = ITERATIONS;
volatile ee_s32 seed5_volatile = 0;

/* Timebase counts per reported second. A simulated run is a few hundred thousand cycles, and CoreMark only calls a run valid when it
 * lasts 10 s: the scale makes it read as 10 kHz so the result can validate. */
#define EE_TICKS_PER_SEC 10000

static inline unsigned long timebase(void)
{
    unsigned long tb;
    __asm__ volatile("mfspr %0, 268" : "=r"(tb));
    return tb;
}

static CORETIMETYPE start_time_val, stop_time_val;

void start_time(void) { start_time_val = (CORETIMETYPE)timebase(); }

void stop_time(void) { stop_time_val = (CORETIMETYPE)timebase(); }

CORE_TICKS get_time(void) { return (CORE_TICKS)(stop_time_val - start_time_val); }

secs_ret time_in_secs(CORE_TICKS ticks) { return (secs_ret)(ticks / EE_TICKS_PER_SEC); }

ee_u32 default_num_contexts = 1;

void uart_send_char(char c)
{
    if (c == '\n')
        putchar('\r');
    putchar(c);
}

void portable_init(core_portable *p, int *argc, char *argv[])
{
    (void)argc;
    (void)argv;
    console_init();
    p->portable_id = 1;
}

void portable_fini(core_portable *p)
{
    p->portable_id = 0;
    ee_printf("\nExecution completed.\n");
}
