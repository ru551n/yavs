// LRM 10.3: a function ends with endfunction
// expect: E4100 8:1 endfunction missing
`timescale 1ns/1ps
module m;
  function f;
    input a;
    f = a;
endmodule
