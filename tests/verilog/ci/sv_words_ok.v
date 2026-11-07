`timescale 1ns / 1ps
// Near miss for E4003: SystemVerilog words in a string and in comments are not SystemVerilog.
// always_ff @(posedge clk) q <= d;  typedef enum { A, B } t;
/* interface foo; endinterface  always_comb x = y; */
module sv_words_ok (input a, output y);
    assign y = a;
    initial $display("always_ff typedef package class endclass");
endmodule
