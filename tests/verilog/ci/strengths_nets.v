`timescale 1ps / 1ps
// Net types and strengths: tri0, tri1, supply0, supply1, drive strengths, and a hierarchical glbl reference.
module strengths_nets (
    input  d, oe,
    inout  pad,
    output y, hi, lo
);
    tri0 glblGSR = glbl.GSR;
    tri1 weak_hi;
    tri (weak1, strong0) pll_lock = weak_hi;
    supply0 gnd;
    supply1 vcc;
    wire (strong1, weak0) drv = d;
    assign (strong1, weak0) y = d;
    assign (pull1, highz0) pad = oe ? d : 1'bz;
    assign hi = vcc;
    assign lo = gnd;
    wire gsr_seen = glbl.GSR | glblGSR;
endmodule
