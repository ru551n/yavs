// LRM 10.4.2: the name of a function is declared inside it, so no other object has that name there
// expect: E4203 7:9 function name again
`timescale 1ns/1ps
module m;
  function f;
    input x;
    reg f;
    f = x;
  endfunction
endmodule
