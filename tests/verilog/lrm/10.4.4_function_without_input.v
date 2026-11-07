// LRM 10.4.4: a function has at least one input argument
// expect: E4100 5:12 no input argument
`timescale 1ns/1ps
module m;
  function f;
    f = 1;
  endfunction
endmodule
