entity runtime_error is end;
architecture t of runtime_error is
  function g(x : integer) return integer is
  begin
    return 10 / x;
  end;
  function h(x : integer) return integer is
  begin
    return g(x - 1) + 1;
  end;
begin
  process
    variable v : integer;
  begin
    wait for 3 ns;
    v := h(1);
    wait;
  end process;
end;
