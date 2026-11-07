// LRM 10.4.4: a function has no nonblocking assignment
// expect: E4100 8:9 nonblocking in a function
module m;
  reg r;
  function f;
    input x;
    begin
      r <= x;
      f = x;
    end
  endfunction
endmodule
