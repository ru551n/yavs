// LRM 10.4.4: a function has no event trigger
// expect: E4100 8:7 trigger in a function
module m;
  event e;
  function f;
    input x;
    begin
      -> e;
      f = x;
    end
  endfunction
endmodule
