// LRM 10.4.4: a function has no event control
// expect: E4100 7:7 event control in a function
module m;
  function f;
    input x;
    begin
      @(x) f = x;
    end
  endfunction
endmodule
