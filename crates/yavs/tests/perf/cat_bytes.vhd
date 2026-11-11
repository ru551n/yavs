-- Strings concatenated in functions that run on native frames: each `Cat` of bytes is one call
-- (`yavs_rt_cat`), not a step of the host on a frame of its own.
entity cat_bytes is end;
architecture a of cat_bytes is
  function img(n : integer) return string is
  begin
    return integer'image(n);
  end function;
  function msg(k : natural) return string is
  begin
    return "[" & img(k) & "] " & img(k * 3) & ".";
  end function;
  function total(n : natural) return natural is
    variable s : natural := 0;
  begin
    for k in 1 to n loop
      s := (s * 7 + msg(k)'length + character'pos(msg(k)(2))) mod 99991;
    end loop;
    return s;
  end function;
begin
  process
  begin
    report "s=" & integer'image(total(200));
    wait;
  end process;
end;
