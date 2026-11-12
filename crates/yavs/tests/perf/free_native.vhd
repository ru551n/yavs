-- Deallocations in a function on a native frame: each is one call of the host's free, not a step
-- of the host (the `new` before it stays a step).
entity free_native is end;
architecture a of free_native is
  type int_ptr is access integer;
  function churn(k, n : integer) return integer is
    variable p : int_ptr;
    variable s : integer := k;
  begin
    for i in 1 to n loop
      p := new integer'(i);
      s := s + p.all;
      deallocate(p);
    end loop;
    return s;
  end function;
begin
  process
    variable s : integer := 0;
  begin
    for k in 0 to 99 loop
      s := (s * 31 + churn(k, 4)) mod 99991;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
