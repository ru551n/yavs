-- difftest: top=psl_temporal_ext std=2008 ref=hand exit=1 ignore=path,order
-- ref=hand: neither reference simulator runs these forms as the LRM defines them (the first stops
-- on or mishandles them, the second has no sync_abort and keeps failing after an until fails);
-- the expected output is the LRM result, derived from the weak finite-trace semantics and
-- checked against an independent evaluator of it. sync_abort (abort and async_abort run
-- the same) inside and outside always, overlapping until and before, a property on the left of until, nested always, next_a over a
-- property, `or` of properties and next_event_e (IEEE 1850 6.2.1.3 to 6.2.1.7).
library ieee;
use ieee.std_logic_1164.all;
entity psl_temporal_ext is end;
architecture t of psl_temporal_ext is
  signal clk : std_logic := '0';
  signal a, b, c : std_logic := '0';
  signal n : integer := 0;
  constant PA : string := "1101001011100110";
  constant PB : string := "0110100110010101";
  constant PC : string := "0011010101001011";
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      if n < PA'length then
        a <= '1' when PA(n + 1) = '1' else '0';
        b <= '1' when PB(n + 1) = '1' else '0';
        c <= '1' when PC(n + 1) = '1' else '0';
      else
        a <= '0'; b <= '0'; c <= '0';
      end if;
    end if;
  end process;

  sabort_in: assert always ((c = '1') -> ((next[2] (b = '1')) sync_abort (a = '1' and b = '1'))) report "sabort_in";
  sabort_out: assert ((always (c = '1' -> next[2] (b = '1'))) sync_abort (a = '1' and b = '1')) report "sabort_out";
  abort_out: assert ((always (c = '1' -> next[2] (b = '1'))) abort (a = '1' and b = '1')) report "abort_out";
  aabort_out: assert ((always (c = '1' -> next[2] (b = '1'))) async_abort (a = '1' and b = '1')) report "aabort_out";
  sabort_a: assert always (c = '1' -> ((next_a[1 to 3] (b = '1')) sync_abort (a = '1' and b = '1'))) report "sabort_a";
  no_abort: assert always (c = '1' -> next[2] (b = '1')) report "no_abort";
  until_ov: assert always (a = '1' -> ((c = '1') until_ (b = '1'))) report "until_ov";
  before_ov: assert always (a = '1' -> ((b = '1') before_ (c = '1'))) report "before_ov";
  until_prop: assert always (a = '1' -> ((next (b = '1')) until (c = '1'))) report "until_prop";
  nested_always: assert always (a = '1' -> next (always (b = '1' -> next (c = '1')))) report "nested_always";
  nexta_prop: assert always (a = '1' -> next_a[1 to 2] (b = '1' -> next (c = '1'))) report "nexta_prop";
  nexte_nevent: assert always (a = '1' -> next_e[1 to 3] (c = '0')) report "nexte_nevent";
  or_props: assert always ((a = '1' -> next (b = '1')) or (a = '1' -> next[2] (c = '1'))) report "or_props";
  neve: assert always (a = '1' -> next_event_e((c = '1'))[2 to 3] (b = '1')) report "neve";
end;
