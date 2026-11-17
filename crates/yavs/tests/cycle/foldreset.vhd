-- Cycle mode: an asynchronous reset whose condition has a constant in an `and` (`rst = '1' and
-- G`, `G and rst = '1'`) is the reset alone when G is true and no reset when it is false: such
-- registers join. A reset of another form and an assert before the edge test keep their process
-- out with a reason that names them (the reason had been "a call of `rising_edge`").
library ieee;
use ieee.std_logic_1164.all;
entity cycle_foldreset_sub is
  generic (W : positive; G : boolean);
  port (clk, rst, en : in std_ulogic; d : in std_ulogic_vector(W - 1 downto 0);
        q1, q2, q3, q4 : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_foldreset_sub is
begin
  first : process (clk, rst)
  begin
    if rst = '1' and G then
      q1 <= (others => '0');
    elsif rising_edge(clk) then
      q1 <= d;
    end if;
  end process;
  second : process (clk, rst)
  begin
    if G and rst = '1' then
      q2 <= (others => '1');
    elsif rising_edge(clk) then
      q2 <= not d;
    end if;
  end process;
  other_form : process (clk, rst, en)
  begin
    if rst = '1' or en = '0' then
      q3 <= (others => '0');
    elsif rising_edge(clk) then
      q3 <= d;
    end if;
  end process;
  assert_first : process (clk)
  begin
    assert W > 0 report "no width";
    if rising_edge(clk) then
      q4 <= d;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_foldreset is
  generic (W : positive := 8);
end entity;
architecture a of cycle_foldreset is
  signal clk, rst, en : std_ulogic := '0';
  signal d : unsigned(W - 1 downto 0) := (others => '0');
  signal q0, q1, q2, q3, q4, q5, q6, q7 : std_ulogic_vector(W - 1 downto 0);
begin
  clk <= not clk after 5 ns;
  on_g : entity work.cycle_foldreset_sub
    generic map (W => W, G => true)
    port map (clk, rst, en, std_ulogic_vector(d), q0, q1, q2, q3);
  off : entity work.cycle_foldreset_sub
    generic map (W => W, G => false)
    port map (clk, rst, en, std_ulogic_vector(d), q4, q5, q6, q7);

  stim : process
  begin
    rst <= '1';
    wait for 2 ns;
    rst <= '0';
    for i in 0 to 15 loop
      wait until falling_edge(clk);
      d <= d + 41 when i mod 3 = 0 else not d;
      en <= '0' when i mod 5 = 4 else '1';
      if i mod 6 = 5 then
        rst <= '1', '0' after 2 ns;
      end if;
      wait for 3 ns;
      report "q=" & to_hstring(q0) & " " & to_hstring(q1) & " " & to_hstring(q2) & " "
        & to_hstring(q3) & " " & to_hstring(q4) & " " & to_hstring(q5) & " "
        & to_hstring(q6) & " " & to_hstring(q7);
    end loop;
    std.env.finish;
  end process;
end architecture;
