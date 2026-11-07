// LRM 7.1: nmos, pmos and their r forms take an output, a data input and a control input; cmos takes four terminals
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, i, n, p;
  nmos m1 (o, i, n);
  pmos m2 (o, i, p);
  rnmos m3 (o, i, n);
  rpmos m4 (o, i, p);
  cmos c1 (o, i, n, p);
  rcmos c2 (o, i, n, p);
endmodule
