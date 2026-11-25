Cycle mode skips a clocked process at an edge when nothing it reads and none of its registers changed since its last
edge, and runs the logic after the registers only where an edge changed one, so an idle block inside a busy design costs
little. yavs watches each clock for this and leaves a clock unwatched for a while where it does not pay; busy designs
pay a little for the watching, and a cold start compiles more region code.
