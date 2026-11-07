// LRM 3.6.3: a string may hold the escapes \n \t \\ \" \ddd and %%
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $display("a\nb\t\\ \" \101 100%%");
endmodule
