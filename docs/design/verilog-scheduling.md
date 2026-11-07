# Verilog scheduling on the VHDL delta cycle

Step A of #776 (plan section 3.5, the "minimally invasive" proposal sections 1 to 3): the Verilog regions of
IEEE 1364-2005 5.4 run inside one kernel process, so the VHDL cycle (`SimControl::cycle`, `update`, `run_batch`)
stays as it is. Code: `crates/yavs-verilog/src/runtime.rs` (regions) and `src/bridge.rs` (the boundary); tests:
`crates/yavs-verilog/tests/sched.rs`. Processes are Rust closures until lowering produces them (step C).

## 1. The domain

One `Domain` holds every Verilog net and reg of the design (one scheduler for all instances, so no module sees
another's update late). It is one kernel process, `serial`: the kernel thread runs it, never a worker.

| Part | What it is |
|---|---|
| Nets and regs | `LogicVec` values with codes `0 1 X Z`, updated at once by a blocking write, with a waiter list each |
| Active | FIFO of processes to run; a write that makes a waiter's item true moves it here |
| Inactive | `#0` waiters; moved to Active when Active is empty |
| NBA | Zero-delay non-blocking updates in issue order (target range and value taken when the assignment ran); applied when Active and Inactive are empty |
| Future | `BTreeMap<Time, Vec<Item>>`: `#d` resumptions and `<= #d` updates in issue order; when their time comes a resumption enters Active and an update the NBA region (a future event keeps its region) |
| Publication queue | Output-port changes as ordered batches, one batch per kernel delta (section 2) |

Edges: `posedge`/`negedge` of element 0 by Table 9-9 (0 to 1, x or z is a posedge, x or z to 1 too; negedge the
mirror; x to z neither), `runtime::edge`. Never `Ctx::edge`, which has the VHDL rules.

## 2. The boundary (decision D4)

Each activation of the domain:

1. Reads every input through `Ctx::logic`, as `TO_X01Z` (L is 0, H is 1, U, W and `-` are x); a value that differs
   from the net is a blocking write (it wakes waiters).
2. Moves the future work due now into its region.
3. Runs the regions until all are empty (or a stop was requested).
4. Assigns batch 0 of the publication queue with zero delay (`Ctx::assign`), so VHDL sees it in the next delta.
5. Waits on every input, and for `Time::ZERO` while batches are left, else until the next future time
   (`Ctx::wait_at`).

Batches: the cursor starts at batch 0 in each activation and moves one batch on at every NBA region, so Active
results reach VHDL at `d+1` and the results of the NBA region and its fanout (continuous assignments of the
updated regs included) at `d+2`. A port written again while an entry of it is in the cursor's batch or a later one
goes to the batch after that entry, and the cursor follows: every change of a port survives, and the order of
changes across ports too. Running a whole time step at once is therefore safe: VHDL sees every intermediate state,
one batch per delta.

Verilog clock into a VHDL flip-flop (`tests/sched.rs` `verilog_clock_and_vhdl_flop`):

| Delta | Visible to VHDL |
|---|---|
| `d` | The domain processes the clock event |
| `d+1` | The clock change of the Active region; a VHDL flip-flop on it samples the old data |
| `d+2` | The register results of the NBA region |

The rule is fixed, not "skip empty batches": with a VHDL clock into a Verilog flip-flop the Active region changes no
output, batch 0 is empty and the register result still reaches VHDL at `d+2` (`vhdl_clock_and_verilog_flop`), one
delta later than a VHDL register on the same clock. A VHDL flip-flop behind a VHDL buffered copy of that clock then
samples the old value. Whether to keep this or skip empty batches (results at `d+1`, like a VHDL register) is an open
question for the maintainer.

Initialisation: inputs start at their VHDL initial value (no event, as in VHDL); every process starts in Active in
declaration order; every output is published at delta 1 with its Active-region value, or its initial value (`x`, `z`)
when the Active region did not change it.

Inputs that change while batches are queued are seen by Verilog in the activation of that delta, after the domain's
own NBA rounds of the earlier delta. Neither LRM defines this cross-language order.

## 3. Why not the kernel's transaction queue

`Waveform::schedule` removes the later transactions of a driver even for transport assignments (LRM 10.5.2.2), so
`q <= #10 1; q <= #5 0;` would lose the update at 10. Future NBAs live in the domain's own map
(`future_nbas_are_not_replaced` shows both behaviours).

## 4. Loops and interrupts

The kernel's delta limit cannot see a loop inside one activation. The domain counts the activations of each process at
one time; past `EVENT_LIMIT` (10000, `Domain::set_event_limit`) the run stops with `E3009` naming the process and the
time. This catches Active ping-pong, `forever #0` and an NBA self-trigger, whatever the design size. Every 1024
activations the loop polls `Ctx::stop_requested` and returns with its queues intact.

## 5. Ceilings

| Ceiling | Upgrade path |
|---|---|
| One serial domain for all Verilog | Independent islands as separate domains, if a profile shows it |
| Every input read and compared per activation | `Ctx::event` first, or per-input wake-ups, when inputs are many |
| Writes element by element | Plane-word copies when lowering makes writes hot |
| Stale waiters pruned when a list doubles | Per-process wait lists if the scan shows in a profile |
| Linear scan of the publication queue per output change | An index per port if batches grow long |
| The event limit is `E3009` (runtime error) | A dedicated code with the diagnostics range of step C |
| Nets do not resolve several drivers; no strengths | Resolution of drivers inside the domain (plan 3.3), then `inout` (R9) |
| The domain decides nothing from VHDL activity (`'active`, transactions without an event) | Not needed by 1364 |
