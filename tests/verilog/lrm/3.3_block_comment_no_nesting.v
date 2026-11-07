// LRM 3.3: block comments do not nest: the first */ ends the comment
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  /* outer /* inner */
  wire w;
endmodule
