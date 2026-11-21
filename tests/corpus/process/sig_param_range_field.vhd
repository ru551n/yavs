-- difftest: top=sig_param_range_field std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A record field of
-- subtype `digit` as the actual of an `integer` formal (LRM 14.7.3, #1181).
entity sig_param_range_field is end entity;
architecture a of sig_param_range_field is
  subtype digit is integer range 0 to 9;
  type rec is record
    d : digit;
    i : integer;
  end record;
  signal r : rec := (0, 0);
  procedure drv (signal o : out integer; x : integer) is begin o <= x; end procedure;
begin
  process
  begin
    drv(r.d, -1);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
