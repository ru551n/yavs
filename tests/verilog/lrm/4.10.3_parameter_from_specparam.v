// LRM 4.10.3: a module parameter is not assigned an expression with a specparam (known gap G6: not checked)
// units: m
// gap: G6 a specparam in a parameter value is accepted
// expect: none
`timescale 1ns/1ps
module m;
  specparam dhold = 1.0;
  parameter regsize = dhold + 1.0;
endmodule
