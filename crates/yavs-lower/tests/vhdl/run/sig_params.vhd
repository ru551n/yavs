-- difftest: top=sig_params std=2008 stop=60ns
entity sig_params is end;
architecture t of sig_params is
  signal clk : bit := '0';
  signal n : integer := 0;
  procedure show(signal s : in integer; tag : string) is
  begin
    report tag & "=" & integer'image(s) & " last=" & integer'image(s'last_value)
      & " event=" & boolean'image(s'event);
  end;
  procedure wait_edge(signal c : in bit) is
  begin
    wait until rising_edge(c);
  end;
  function pick(k : natural) return integer is
    type tab is array (0 to 3) of integer;
    constant t : tab := (5, k, 7, others => -1);
    variable w : tab := (2 => 7, 0 => k, 1 to 1 => 3, others => -1);
    variable v : bit_vector(3 downto 0) := (1 => '1', others => '0');
  begin
    v(3) := '1';
    return t(k) + w(k) * 1000 + bit'pos(v(3)) + bit'pos(v(1)) * 10 + bit'pos(v(0)) * 100;
  end;
begin
  clk <= not clk after 5 ns;
  with n select
    n <= n + 1 after 1 ns when 0 | 1,
         n + 2 after 1 ns when 2,
         n after 0 ns when others;
  process
  begin
    for i in 1 to 3 loop
      wait_edge(clk);
      show(n, "edge" & integer'image(i));
    end loop;
    report "pick " & integer'image(pick(0)) & " " & integer'image(pick(1)) & " "
      & integer'image(pick(2)) & " " & integer'image(pick(3));
    wait;
  end process;
end;
