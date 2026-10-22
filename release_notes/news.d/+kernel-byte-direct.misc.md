Designs with enumeration, integer or boolean registers (state machines, counters) simulate faster: an assignment to such a signal skips the event queue and costs about a third less in the kernel.
