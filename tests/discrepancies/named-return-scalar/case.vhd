-- LRM 2019 4.2.1: a named return value whose type mark denotes a scalar type; `r` is the
-- target's subtype, so `r'high` is the high bound of the target's range.
entity top is end entity;
architecture a of top is
  subtype small is integer range 3 to 9;
  function hi return r of integer is
  begin
    return r'high;
  end function;
begin
  process
    variable v : small;
  begin
    v := hi;
    report "RES hi " & integer'image(v);
    wait;
  end process;
end architecture;
