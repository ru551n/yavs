`timescale 1ns / 1ps
// Combinational UDP with don't-care and X rows, and a user of it.
primitive udp_mux2 (o, i0, i1, s);
    output o;
    input i0, i1, s;
    table
    // i0 i1 s : o
       0  ?  0 : 0 ;
       1  ?  0 : 1 ;
       ?  0  1 : 0 ;
       ?  1  1 : 1 ;
       0  0  x : 0 ;
       1  1  x : 1 ;
    endtable
endprimitive

module udp_comb_user (input a, b, s, output y);
    udp_mux2 u (y, a, b, s);
endmodule
