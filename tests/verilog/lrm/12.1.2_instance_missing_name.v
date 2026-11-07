// LRM 12.1.2: an instance of a module has an instance name; only gate and UDP instances may omit it (known gap G1: the parser cannot tell a module from a UDP before elaboration)
// units: c,m
// gap: G1 an unnamed module instance is accepted until elaboration knows the kind of c
// expect: none
`timescale 1ns/1ps
module c;
endmodule
module m;
  c ();
endmodule
