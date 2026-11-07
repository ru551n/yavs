// LRM 7.1: bufif and notif take an output, a data input and a control input
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, i, en;
  bufif0 b0 (o, i, en);
  bufif1 b1 (o, i, en);
  notif0 n0 (o, i, en);
  notif1 n1 (o, i, en);
endmodule
