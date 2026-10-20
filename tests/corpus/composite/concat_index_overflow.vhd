-- difftest: top=concat_index_overflow std=2008 exit=1 ignore=fatalmsg
-- The result of `&` does not fit the index subtype of its array type: a runtime error naming
-- the operator (LRM 9.2.5), also when a piece is a single element.
-- (The message wording differs between simulators: only the failure and its place count.)
entity concat_index_overflow is end;
architecture t of concat_index_overflow is
  type small is range 0 to 5;
  type sv is array (small range <>) of bit;
  function zero(x : integer) return small is
  begin
    return small(x - x);
  end;
begin
  process
    variable a : sv(0 to 3) := "1010";
    variable b : sv(0 to 1) := "11";
    variable c : sv(0 to zero(1)) := "0";
    variable r : sv(0 to 5);
  begin
    r := a & b;
    report "six elements fit: " & integer'image(r'length);
    r := a & b & c;
    report "not reached";
    wait;
  end process;
end;
