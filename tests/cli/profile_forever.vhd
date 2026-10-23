-- `yavs run --profile` then Ctrl-C: a design that never ends.
entity profile_forever is end;
architecture t of profile_forever is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  hot : process (clk)
    variable a : integer := 0;
  begin
    for i in 1 to 2000 loop
      a := (a * 3 + i) mod 10007;
    end loop;
  end process;
end;
