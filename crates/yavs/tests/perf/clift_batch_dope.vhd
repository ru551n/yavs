-- Element drives of a vector whose bounds come from a generic, in a loop under compiled code:
-- the descriptor reads between the drives keep them in one batch (one kernel call an activation).
library ieee; use ieee.std_logic_1164.all;
entity clift_batch_dope_core is
  generic (n : positive);
  port (clk : in std_logic; q : out std_logic_vector(n - 1 downto 0));
end;
architecture a of clift_batch_dope_core is
  signal s : std_logic_vector(n - 1 downto 0) := (0 => '1', others => '0');
begin
  q <= s;
  process (clk) begin
    if rising_edge(clk) then
      for i in 1 to s'high loop
        s(i) <= s(i - 1) xor s(s'high);
      end loop;
      s(0) <= s(s'high);
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity clift_batch_dope is end;
architecture a of clift_batch_dope is
  signal clk : std_logic := '0';
  signal a : std_logic_vector(11 downto 0);
  signal b : std_logic_vector(15 downto 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  u0 : entity work.clift_batch_dope_core generic map (n => 12) port map (clk, a);
  u1 : entity work.clift_batch_dope_core generic map (n => 16) port map (clk, b);
  process begin
    wait for 500 ns;
    report "q=" & to_hstring(a) & " " & to_hstring(b);
    done <= true; wait;
  end process;
end;
