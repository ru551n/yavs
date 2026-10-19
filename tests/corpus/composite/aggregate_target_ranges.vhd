-- difftest: top=aggregate_target_ranges std=2008
-- Range choices in variable aggregate targets (#244, LRM 10.6.2.1, lrm-checks L68): the
-- aggregate runs over its choices in the direction of its range choices, its leftmost
-- index takes the value's leftmost element, and each piece takes the elements of its range
-- in that order. Ascending and descending targets and values, mixed element and range
-- choices, subtype names and range attributes as choices.
entity aggregate_target_ranges is end;
architecture t of aggregate_target_ranges is
  type iv is array (natural range <>) of integer;
  subtype didx is integer range 7 downto 0;
  type dv is array (didx range <>) of integer;
  subtype hr is natural range 5 downto 2;
  function img(x : iv) return string is
  begin
    if x'length = 0 then return ""; end if;
    if x'ascending then
      return " " & integer'image(x(x'left)) & img(x(x'left + 1 to x'right));
    end if;
    return " " & integer'image(x(x'left)) & img(x(x'left - 1 downto x'right));
  end function;
begin
  process
    variable vd : iv(5 downto 0) := (50, 40, 30, 20, 10, 0);
    variable va : iv(0 to 5) := (0, 10, 20, 30, 40, 50);
    variable hi : iv(5 downto 2);
    variable lo : iv(1 downto 0);
    variable a : iv(0 to 1);
    variable b : iv(2 to 5);
    variable h0 : iv(0 to 3);
    variable m : iv(4 downto 2);
    variable e0, e1, e5 : integer;
    variable m5 : iv(1 to 5);
    variable d : dv(5 downto 0) := (50, 40, 30, 20, 10, 0);
    variable da : dv(0 to 1);
    variable db : dv(2 to 5);
  begin
    (5 downto 2 => hi, 1 downto 0 => lo) := vd;
    report "desc/desc hi=" & img(hi) & " lo=" & img(lo);
    assert hi = (50, 40, 30, 20) and lo = (10, 0) severity failure;
    (0 to 1 => a, 2 to 5 => b) := vd;
    report "asc/desc a=" & img(a) & " b=" & img(b);
    assert a = (50, 40) and b = (30, 20, 10, 0) severity failure;
    (5 downto 2 => hi, 1 downto 0 => lo) := va;
    report "desc/asc hi=" & img(hi) & " lo=" & img(lo);
    assert hi = (0, 10, 20, 30) and lo = (40, 50) severity failure;
    (0 to 1 => a, 2 to 5 => b) := va;
    report "asc/asc a=" & img(a) & " b=" & img(b);
    assert a = (0, 10) and b = (20, 30, 40, 50) severity failure;
    (9 downto 6 => h0, 5 downto 4 => a) := vd;
    report "shifted h0=" & img(h0) & " a=" & img(a);
    assert h0 = (50, 40, 30, 20) and a = (10, 0) severity failure;
    (5 => e5, 4 downto 2 => m, 1 => e1, 0 => e0) := vd;
    report "mixed e5=" & integer'image(e5) & " m=" & img(m) & " e1=" & integer'image(e1)
      & " e0=" & integer'image(e0);
    assert e5 = 50 and m = (40, 30, 20) and e1 = 10 and e0 = 0 severity failure;
    (0 => e0, 1 to 5 => m5) := vd;
    report "element first e0=" & integer'image(e0) & " m5=" & img(m5);
    assert e0 = 50 and m5 = (40, 30, 20, 10, 0) severity failure;
    (hr => hi, lo'range => lo) := va;
    report "names hi=" & img(hi) & " lo=" & img(lo);
    assert hi = (0, 10, 20, 30) and lo = (40, 50) severity failure;
    (0 to 1 => da, 2 to 5 => db) := d;
    report "descending index subtype da=" & integer'image(da(0)) & " " & integer'image(da(1));
    assert da = (50, 40) and db = (30, 20, 10, 0) severity failure;
    (5 downto 0 => va) := vd;
    report "one range va=" & img(va);
    assert va = (50, 40, 30, 20, 10, 0) severity failure;
    wait;
  end process;
end;
