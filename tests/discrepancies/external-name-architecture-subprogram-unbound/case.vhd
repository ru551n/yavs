-- LRM 8.7, 14.4.1: an external name in a subprogram of an architecture, naming nothing, in a
-- subprogram that is never called.
entity top is end;
architecture t of top is
  impure function never return integer is
  begin
    return <<constant .top.nope : integer>>;
  end function;
begin
  process
  begin
    report "RES ok";
    wait;
  end process;
end;
