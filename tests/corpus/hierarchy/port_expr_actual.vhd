-- difftest: top=port_expr_actual std=2008 stop=20ns
-- Non-static expression actuals of in ports (LRM 6.5.6.3): each is driven by an equivalent
-- process `formal <= expr` in the instantiating region, sensitive to the expression's
-- signals, so the formal follows the expression one delta later. Scalars, arrays, a function
-- call, a formal sub-element of an individual association, and an instance in a generate.
entity mon is
  generic (tag : string; idx : natural := 0);
  port (n : in integer := -1; v : in bit_vector(3 downto 0) := "1010");
end;
architecture a of mon is
begin
  process (n, v) begin
    report tag & integer'image(idx) & " n=" & integer'image(n) & " v=" & to_string(v) & " @" & time'image(now);
  end process;
end;
entity port_expr_actual is end;
architecture a of port_expr_actual is
  signal x, y : integer := 1;
  signal b : bit_vector(3 downto 0) := "0011";
  function twice(i : integer) return integer is begin return 2 * i; end;
begin
  m1 : entity work.mon generic map ("m1") port map (n => x + y, v => b and "0101");
  m2 : entity work.mon generic map ("m2") port map (n => twice(x), v => not b);
  m3 : entity work.mon generic map ("m3")
    port map (n => y, v(3 downto 2) => b(1 downto 0), v(1 downto 0) => b(3 downto 2) xor "11");
  g : for k in 1 to 2 generate
    m : entity work.mon generic map ("g", k) port map (n => x * k);
  end generate;
  process begin
    wait for 1 ns;
    x <= 5;
    wait for 1 ns;
    b <= "1100";
    y <= 10;
    wait for 1 ns;
    assert x + y = 15 severity failure;
    wait;
  end process;
end;
