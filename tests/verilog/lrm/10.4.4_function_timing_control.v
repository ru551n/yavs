// LRM 10.4.4: a function has no timing-controlled statement
// expect: E4100 7:7 delay in a function
module m;
  function f;
    input x;
    begin
      #1 f = x;
    end
  endfunction
endmodule
