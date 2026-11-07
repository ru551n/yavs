// LRM 14.5: pulsestyle and showcancelled declarations name outputs
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    pulsestyle_onevent q;
    pulsestyle_ondetect q;
    showcancelled q;
    noshowcancelled q;
    (a => q) = 1;
  endspecify
endmodule
