-- difftest: top=psl_temporal std=2008 ref=hand exit=1 ignore=path,order
-- ref=hand: the first reference simulator stops on or mishandles most of these operators; the
-- result is the LRM one, which the second reference simulator gives (checked) except where a
-- directive is marked. Weak temporal operators (IEEE 1850 6.2.1.3 to 6.2.1.5, 6.2.1.7): a
-- failure is reported at the tick the obligation can no longer hold, once per tick.
library ieee;
use ieee.std_logic_1164.all;
entity psl_temporal is end;
architecture t of psl_temporal is
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

  nexta:  assert always (a = '1' -> next_a[1 to 3] (b = '1')) report "next_a";
  nexte:  assert always (a = '1' -> next_e[1 to 3] (b = '1')) report "next_e";
  nev1:   assert always (a = '1' -> next_event(c = '1')(b = '1')) report "next_event";
  nev2:   assert always (a = '1' -> next_event(c = '1')[2](b = '1')) report "next_event[2]";
  neva:   assert always (a = '1' -> next_event_a(c = '1')[1 to 2](b = '1')) report "next_event_a";
  neve:   assert always (a = '1' -> next_event_e(c = '1')[1 to 2](b = '1')) report "next_event_e";
  bef:    assert always (a = '1' -> (b = '1' before c = '1')) report "before";
  iff1:   assert always (a = '1' <-> b = '1') report "iff";
  until1: assert always (a = '1' -> (c = '0' until b = '1')) report "until";
  nextnx: assert always (a = '1' -> next (b = '1' -> next[2] (c = '1'))) report "nested next";
  andp:   assert always ((a = '1' -> next (b = '1')) and (a = '1' -> next[2] (c = '1'))) report "and";
  suf1:   assert always {a = '1'; b = '1'} |=> next[2] (c = '1') report "suffix next";
  suf2:   assert always {a = '1'} |-> next_a[1 to 2] (b = '1') report "suffix next_a";
  suf3:   assert always {a = '1'} |=> next_e[1 to 3] (c = '1') report "suffix next_e";
  suf4:   assert always {a = '1'; a = '1'} |=> (b = '1' before c = '1') report "suffix before";
  suf5:   assert always {a = '1'} |-> next_event(c = '1')[2](b = '1') report "suffix next_event";
  suf6:   assert always {a = '1'} |=> ({b = '1'; c = '1'}) report "suffix sequence";
  first1: assert (a = '1' -> next_a[1 to 3] (b = '1')) report "first tick";
end;
