// LRM 10.3: a function has input arguments only
// expect: E4100 5:12 output argument of a function
`timescale 1ns/1ps
module m;
  function f;
    input a;
    output b;
    f = a;
  endfunction
endmodule
