-- A null slice returned by a function keeps its own bounds (LRM 8.5).
entity top is end;
architecture a of top is
  function cut(x : bit_vector; l, r : integer) return bit_vector is
  begin
    return x(l to r);
  end;
  function bounds(x : bit_vector) return string is
  begin
    return integer'image(x'left) & " " & integer'image(x'right) & " " & integer'image(x'length);
  end;
begin
  process
    variable v : bit_vector(0 to 7);
  begin
    report "RES " & bounds(cut(v, 7, 2));
    wait;
  end process;
end;
