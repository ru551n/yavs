// LRM 10.4.4: a function has no wait statement
// expect: E4100 6:5 wait in a function
module m;
  function f;
    input x;
    wait (x) f = x;
  endfunction
endmodule
