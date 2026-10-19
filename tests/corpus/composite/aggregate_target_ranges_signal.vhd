-- difftest: top=aggregate_target_ranges_signal std=2008
-- Range choices in signal aggregate targets (#244, LRM 10.5.2.1, lrm-checks L68), sequential
-- and concurrent, with the element mapping of variable targets.
entity aggregate_target_ranges_signal is end;
architecture t of aggregate_target_ranges_signal is
  type iv is array (natural range <>) of integer;
  function img(x : iv) return string is
  begin
    if x'length = 0 then return ""; end if;
    if x'ascending then
      return " " & integer'image(x(x'left)) & img(x(x'left + 1 to x'right));
    end if;
    return " " & integer'image(x(x'left)) & img(x(x'left - 1 downto x'right));
  end function;
  signal vd : iv(5 downto 0) := (50, 40, 30, 20, 10, 0);
  signal va : iv(0 to 5) := (0, 10, 20, 30, 40, 50);
  signal hi, chi : iv(5 downto 2);
  signal lo, clo : iv(1 downto 0);
  signal a : iv(0 to 1);
  signal b : iv(2 to 5);
  signal bits : bit_vector(5 downto 0) := "110010";
  signal bh : bit_vector(3 downto 0);
  signal bl : bit_vector(0 to 1);
begin
  (5 downto 2 => chi, 1 downto 0 => clo) <= vd;
  (0 to 3 => bh, 4 to 5 => bl) <= bits after 1 ns;
  process
  begin
    (5 downto 2 => hi, 1 downto 0 => lo) <= vd;
    (0 to 1 => a, 2 to 5 => b) <= vd;
    wait for 1 ns;
    report "desc hi=" & img(hi) & " lo=" & img(lo) & " a=" & img(a) & " b=" & img(b);
    assert hi = (50, 40, 30, 20) and lo = (10, 0) severity failure;
    assert a = (50, 40) and b = (30, 20, 10, 0) severity failure;
    report "concurrent hi=" & img(chi) & " lo=" & img(clo);
    assert chi = (50, 40, 30, 20) and clo = (10, 0) severity failure;
    report "bits bh=" & to_string(bh) & " bl=" & to_string(bl);
    assert bh = "1100" and bl = "10" severity failure;
    (5 downto 2 => hi, 1 downto 0 => lo) <= va, vd after 2 ns;
    wait for 1 ns;
    report "asc hi=" & img(hi) & " lo=" & img(lo);
    assert hi = (0, 10, 20, 30) and lo = (40, 50) severity failure;
    wait for 2 ns;
    report "later hi=" & img(hi) & " lo=" & img(lo);
    assert hi = (50, 40, 30, 20) and lo = (10, 0) severity failure;
    vd <= (5, 4, 3, 2, 1, 0);
    wait for 1 ns;
    report "concurrent again hi=" & img(chi) & " lo=" & img(clo);
    assert chi = (5, 4, 3, 2) and clo = (1, 0) severity failure;
    wait;
  end process;
end;
