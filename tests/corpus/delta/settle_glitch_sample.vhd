-- difftest: top=settle_glitch_sample std=2008 stop=40ns
-- Values in the middle of a time step seen by a clock derived through processes, by
-- 'transaction and by 'last_value and 'last_event in the next step (#1064).
entity settle_glitch_sample is end;
architecture t of settle_glitch_sample is
  signal clk, c1, c2, c3 : bit := '0';
  signal a, a_d, z : bit := '0';
  signal n : natural := 0;
begin
  clk <= not clk after 5 ns;
  b1 : process (clk) begin c1 <= clk; end process;
  b2 : process (c1) begin c2 <= c1; end process;
  b3 : process (c2) begin c3 <= c2; end process;
  src : process (clk) begin
    if clk'event and clk = '1' then a <= not a; end if;
  end process;
  dly : process (a) begin a_d <= a; end process;
  glitch : process (a, a_d) begin z <= a xor a_d; end process;
  sample : process (c3) begin
    if c3'event and c3 = '1' then
      report "z at the derived edge " & to_string(z);
    end if;
  end process;
  count : process (z'transaction) begin n <= n + 1; end process;
  hist : process begin
    wait for 7 ns;
    loop
      report "n=" & integer'image(n) & " last=" & to_string(z'last_value)
        & " since=" & time'image(z'last_event);
      wait for 10 ns;
    end loop;
  end process;
end;
