-- Cycle mode gate design: a finite state machine whose state is a record (an enumeration, a counter
-- and a flag), in the two-process style: a combinational process computes the next state on a
-- variable copy, a clocked process registers it. The testbench drives the inputs and prints the
-- state at every rising edge and 1 ns after it.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_fsmrec is
end entity;
architecture a of cycle_fsmrec is
  type state_t is (IDLE, RUN, WAIT_ACK, DONE, FAULT);
  type fsm_t is record
    st  : state_t;
    cnt : unsigned(3 downto 0);
    err : std_ulogic;
  end record;
  constant RESET : fsm_t := (st => IDLE, cnt => (others => '0'), err => '0');
  signal clk, rst, go, ack : std_ulogic := '0';
  signal s, nxt : fsm_t := RESET;
  signal busy, pulse : std_ulogic := '0';
  impure function snap return string is
  begin
    return state_t'image(s.st) & " cnt " & to_string(to_integer(s.cnt)) & " err " & to_string(s.err)
      & " busy " & to_string(busy) & " pulse " & to_string(pulse) & " go " & to_string(go) & " ack " & to_string(ack);
  end function;
begin
  clk <= not clk after 5 ns;

  next_state : process (s, go, ack)
    variable v : fsm_t;
  begin
    v := s;
    case s.st is
      when IDLE =>
        v.cnt := (others => '0');
        if go = '1' then
          v.st := RUN;
        end if;
      when RUN =>
        v.cnt := s.cnt + 1;
        if s.cnt = 5 then
          v.st := WAIT_ACK;
        end if;
      when WAIT_ACK =>
        v.cnt := s.cnt + 1;
        if ack = '1' then
          v.st := DONE;
        elsif s.cnt = 12 then
          v.st := FAULT;
          v.err := '1';
        end if;
      when DONE =>
        v.st := IDLE;
      when FAULT =>
        if go = '0' then
          v.st := IDLE;
          v.err := '0';
        end if;
    end case;
    nxt <= v;
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        s <= RESET;
      else
        s <= nxt;
      end if;
    end if;
  end process;

  busy <= '0' when s.st = IDLE or s.st = DONE else '1';
  pulse <= '1' when s.st = DONE else '0';

  -- two waits: stays outside the region; reads the values before the edge, then after it
  monitor : process
  begin
    wait until rising_edge(clk);
    report "pre  " & snap;
    wait for 1 ns;
    report "post " & snap;
  end process;

  stim : process
  begin
    rst <= '1';
    wait until falling_edge(clk);
    wait until falling_edge(clk);
    rst <= '0';
    for i in 0 to 99 loop
      go <= '1' when (i mod 17) < 9 else '0';
      ack <= '1' when i mod 11 = 8 or i mod 13 = 5 else '0';
      rst <= '1' when i = 70 else '0';
      wait until falling_edge(clk);
    end loop;
    wait for 1 ns;
    std.env.stop;
  end process;
end architecture;
