-- difftest: top=slice_target_bound_once std=2008 ref=hand
-- ref=hand: a slice target whose bound is an impure call, with a value that takes its bounds
-- from the target (an aggregate): the name is evaluated once (LRM 10.6.2.1), so the call runs
-- once (#1157; one reference simulator runs it three or four times). Variable targets only: a
-- signal target still runs it twice (#1157).
entity slice_target_bound_once is
end entity;

architecture sim of slice_target_bound_once is
  type iv is array (natural range <>) of integer;
begin
  process is
    variable v, w : iv(0 to 3) := (others => 0);
    variable n : natural := 0;
    impure function t(k : natural) return natural is
    begin
      n := n + 1;
      return k;
    end function;
  begin
    n := 0;
    v(t(1) to 2) := w(0 to 1);
    report "plain value: " & integer'image(n);
    n := 0;
    v(t(1) to 2) := (5, 6);
    report "positional aggregate: " & integer'image(n) & " " & integer'image(v(1)) & integer'image(v(2));
    n := 0;
    v(t(0) to 3) := (others => 7);
    report "others: " & integer'image(n) & " " & integer'image(v(0)) & integer'image(v(3));
    wait;
  end process;
end architecture;
