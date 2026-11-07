`timescale 1ns / 1ps
// Switch-level primitives parse and analyse; they are refused when a design reaches them.
module unsupported_switch (inout a, inout b, input g, gn, output y);
    nmos  n1 (y, a, g);
    pmos  p1 (y, a, gn);
    cmos  c1 (y, a, g, gn);
    tran  t1 (a, b);
    tranif1 t2 (a, b, g);
    rtran rt (a, b);
endmodule
