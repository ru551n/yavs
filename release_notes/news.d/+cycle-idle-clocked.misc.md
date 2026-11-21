Cycle mode skips a clocked process at an edge when nothing it reads and none of its registers changed since its last
edge, and runs the logic after the registers only where an edge changed one, so an idle block inside a busy design costs
little; a clock where this does not pay is left unwatched for a while.
