// LRM 3.8: an attribute instance precedes an item or statement (known gap G2: a dangling one before endmodule is accepted)
// units: m
// gap: G2 an attribute instance that precedes nothing is accepted
// expect: none
`timescale 1ns/1ps
module m;
  wire w;
  (* orphan *)
endmodule
