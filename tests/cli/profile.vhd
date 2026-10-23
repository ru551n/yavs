-- `yavs run --profile`: process `hot` (line 16, its loop in `burn`, line 10) does almost all the work, `cold` almost none.
entity profile is end;
architecture t of profile is
  signal clk : bit := '0';
  signal n   : integer := 0;
  function burn(x : integer) return integer is
    variable a : integer := x;
  begin
    for i in 1 to 2000 loop
      a := (a * 3 + i) mod 10007;
    end loop;
    return a;
  end function;
begin
  clk <= not clk after 5 ns when now < 40 us else clk;
  hot : process (clk)
    variable v : integer := 0;
  begin
    if clk = '1' then
      v := burn(v);
      n <= v;
    end if;
  end process;
  cold : process (clk)
  begin
    if clk = '1' then
      assert n >= 0;
    end if;
  end process;
end;
