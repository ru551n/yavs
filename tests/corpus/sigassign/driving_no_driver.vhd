-- difftest: top=driving_no_driver std=2008 exit=1 ref=hand
-- 'driving of a signal parameter whose actual the process does not drive (LRM 16.2.4).
-- ref=hand: the expected text is yavs's wording of the run-time error.
entity driving_no_driver is end entity;
architecture a of driving_no_driver is
  type rec is record a, b : bit; end record;
  signal r : rec;
  procedure p (signal y : in rec) is begin report boolean'image(y.a'driving); end procedure;
begin
  r.a <= '1';
  process begin wait for 1 ns; p(r); wait; end process;
end architecture;
