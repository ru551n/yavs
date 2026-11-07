`timescale 1ns / 1ps
// Sequential UDPs: level-sensitive latch and edge-sensitive flop, with an initial value.
primitive udp_latch (q, d, en);
    output q;
    reg q;
    input d, en;
    table
    // d en : q : q+
       1 1  : ? : 1 ;
       0 1  : ? : 0 ;
       ? 0  : ? : - ;
    endtable
endprimitive

primitive udp_dff (q, clk, d, clr);
    output q;
    reg q;
    input clk, d, clr;
    initial q = 1'b0;
    table
    // clk   d clr : q : q+
       r     0  0  : ? : 0 ;
       r     1  0  : ? : 1 ;
       (01)  0  0  : ? : 0 ;
       (0x)  1  0  : 1 : 1 ;
       n     ?  0  : ? : - ;
       ?     *  0  : ? : - ;
       ?     ?  1  : ? : 0 ;
    endtable
endprimitive

module udp_seq_user (input clk, d, clr, en, output q1, q2);
    udp_latch l (q1, d, en);
    udp_dff f (q2, clk, d, clr);
endmodule
