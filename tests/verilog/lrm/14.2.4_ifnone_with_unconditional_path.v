// LRM 14.2.4: a path has an ifnone path or an unconditional simple path, not both (known gap G5: not checked)
// units: m
// gap: G5 ifnone and an unconditional path for one path are accepted
// expect: none
`timescale 1ns/1ps
module m (input a, c, output q);
  specify
    if (c) (a => q) = 1;
    ifnone (a => q) = 2;
    (a => q) = 3;
  endspecify
endmodule
