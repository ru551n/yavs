// LRM 10.4.4: a function has no procedural continuous assignment
// expect: E4100 8:7 assign in a function
module m;
  reg r;
  function f;
    input x;
    begin
      assign r = x;
      f = x;
    end
  endfunction
endmodule
