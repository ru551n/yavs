// LRM 3.3: inside a block comment // has no meaning and the comment still ends at */
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  /* // still inside */ wire w;
endmodule
