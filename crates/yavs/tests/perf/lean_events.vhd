-- Wide buses and the attributes asked of them: `a` and `b` are only read as a whole ('event,
-- 'last_event of the whole bus, 'active) or by element (edge tests, 'last_value of a bit), so the
-- kernel keeps just each block's last event for them; `c` has an 'last_event of one element and
-- keeps the event of every element. The report is the same either way (YAVS_LEAN_EVENTS=off).
library ieee; use ieee.std_logic_1164.all;
entity lean_events is end;
architecture a of lean_events is
  signal clk : std_logic := '0';
  signal a : std_logic_vector(199 downto 0) := (0 => '1', 7 => '1', others => '0');
  signal b : std_logic_vector(129 downto 0) := (others => '0');
  signal c : std_logic_vector(99 downto 0) := (3 => '1', 9 => '1', others => '0');
  signal n : natural := 0;
  signal done : boolean := false;
  function lev(t : time) return integer is
  begin
    if t = time'high then return -1; end if;
    return t / 1 ns;
  end function;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      a <= a(198 downto 0) & (a(199) xor a(5));
      if n mod 3 = 0 then b <= b(128 downto 0) & not b(129); end if;
      if n mod 4 = 0 then c <= c(94 downto 0) & c(99 downto 95); end if;
    end if;
  end process;
  process
    variable ea, eb, ec, ra, fa : natural := 0;
  begin
    while not done loop
      wait on a, b, c, clk;
      if a'event then ea := ea + 1; end if;
      if b'event then eb := eb + 1; end if;
      if c'event then ec := ec + 1; end if;
      if rising_edge(a(7)) then ra := ra + 1; end if;
      if falling_edge(a(7)) then fa := fa + 1; end if;
    end loop;
    report "events=" & integer'image(ea) & "," & integer'image(eb) & "," & integer'image(ec)
      & " edges=" & integer'image(ra) & "," & integer'image(fa)
      & " last_event=" & integer'image(lev(a'last_event)) & "," & integer'image(lev(b'last_event)) & ","
      & integer'image(lev(c(68)'last_event)) & "," & integer'image(lev(c(3)'last_event))
      & " last_value=" & std_logic'image(a(7)'last_value) & std_logic'image(c(3)'last_value)
      & " active=" & boolean'image(a'last_active = 0 ns);
    wait;
  end process;
  process begin
    wait for 1 us;
    done <= true; wait;
  end process;
end;
