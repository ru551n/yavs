`timescale 1ns / 1ps
// Gate primitives: delays, drive strengths, instance arrays, tri-state, pullup and pulldown.
module gates (
    input a, b, c, oe,
    input [3:0] v,
    output y_and, y_or, y_not, y_buf, y_xor, y_nand,
    output [3:0] arr,
    inout bus
);
    and  #(1, 2) g1 (y_and, a, b, c);
    or   #3 (y_or, a, b);
    not  (y_not, a);
    buf  (strong1, weak0) (y_buf, a);
    xor  g4 (y_xor, a, b);
    nand #(1:2:3, 2:3:4) g5 (y_nand, a, b);
    not  n[3:0] (arr, v);
    bufif1 t1 (bus, a, oe);
    notif0 t2 (bus, b, oe);
    pullup (bus);
    pulldown pd (y_buf);
endmodule
