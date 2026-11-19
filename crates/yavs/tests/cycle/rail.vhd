-- 'U' at start under --cycle-exact-meta (owner decision A38): registers without a start value
-- (a pipeline of three, `hi` whose high word stays 'U', `held` that keeps 'U' past the rail's
-- cap), a clocked variable without one, and an input that becomes 'U' after time 0, read by a
-- testbench that waits on `s3` and reads the rest without waiting on them. Without KEEP `held`
-- follows `d` from the first edge and the second image ends when it agrees with the region's.
library ieee; use ieee.std_logic_1164.all;
entity cycle_rail is
  generic (W : positive := 8; KEEP : boolean := true);
end;
architecture a of cycle_rail is
  signal clk, en, en2, en3 : std_ulogic := '0';
  signal d : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  signal din : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  signal s1, s2, s3, hi, cp, fromvar, held : std_ulogic_vector(W - 1 downto 0);
  -- `kept` keeps its start value while `held` is 'U' (the normal mode takes no branch on it).
  signal kept : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  -- BIT and BOOLEAN are logic in the kernel too, but never 'U': `bp` computes them from `s2`.
  type pair is record
    l : std_ulogic;
    b : boolean;
  end record;
  signal bo : boolean;
  signal bt : bit;
  signal rc : pair;
begin
  clk <= not clk after 5 ns when now < 12 us else clk;
  pipe : process (clk) begin
    if rising_edge(clk) then
      if en = '1' then
        s1 <= d;
      end if;
      s2 <= s1;
      s3 <= s2;
    end if;
  end process;
  hp : process (clk) begin
    if rising_edge(clk) then
      for i in 0 to W - 1 loop
        if i < 64 or en2 = '1' then
          hi(i) <= not d(i);
        end if;
      end loop;
    end if;
  end process;
  vp : process (clk)
    variable v : std_ulogic_vector(W - 1 downto 0);
  begin
    if rising_edge(clk) then
      fromvar <= v;
      if en2 = '1' then
        v := d;
      end if;
    end if;
  end process;
  cpp : process (clk) begin
    if rising_edge(clk) then
      cp <= din;
    end if;
  end process;
  hold : process (clk) begin
    if rising_edge(clk) then
      if en3 = '1' or not KEEP then
        held <= d;
      end if;
      if held(0) = '1' then
        kept <= d;
      end if;
    end if;
  end process;
  bp : process (clk) begin
    if rising_edge(clk) then
      bo <= s2(0) = '1';
      bt <= to_bit(s2(0));
      rc <= (l => s2(0), b => s2(0) = '1');
    end if;
  end process;
  mon : process (s3) begin
    report "s3=" & to_string(s3);
  end process;
  stim : process
    procedure show (tag : string) is
    begin
      report tag & " s1=" & to_string(s1) & " s2=" & to_string(s2) & " hi=" & to_string(hi)
        & " cp=" & to_string(cp) & " fromvar=" & to_string(fromvar) & " bo=" & boolean'image(bo)
        & " bt=" & bit'image(bt) & " rc=" & std_ulogic'image(rc.l) & boolean'image(rc.b)
        & " kept=" & to_string(kept) & " held=" & to_string(held);
    end procedure;
  begin
    wait for 2 ns; show("a");
    wait for 20 ns; show("b");
    en <= '1';
    wait for 10 ns; show("c");
    wait for 10 ns; show("d");
    wait for 10 ns; show("e");
    en2 <= '1';
    wait for 10 ns; show("f");
    wait for 10 ns; show("g");
    wait for 40 ns; din <= (others => 'U');
    wait for 12 ns; show("h");
    wait for 100 ns; din <= (others => '1');
    wait for 12 ns; show("i");
    wait for 10 us - now; show("j");
    wait for 1 us; show("k");
    wait;
  end process;
end;
