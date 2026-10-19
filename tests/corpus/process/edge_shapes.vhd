-- difftest: top=edge_shapes std=2008
-- Clocked processes of every shape the kernel can leave suspended between edges: an element
-- of a vector, a clock next to other signals in the sensitivity list, `rising_edge(c) and X`,
-- and the wait forms. Weak and unknown levels ('H', 'L', 'X', 'U') must give the same edges
-- as without the shortcut; each counter is checked against the count made by hand.
library ieee;
use ieee.std_logic_1164.all;

entity edge_shapes is end;

architecture t of edge_shapes is
  signal clk : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := "0000";
  signal en, rst : std_logic := '0';
  signal c_up, c_dn, c_en, c_wu, c_wd, c_wi, c_sl : natural := 0;
  type rec is record a, b : std_logic; end record;
  signal r : rec := ('0', '0');
  signal c_rec : natural := 0;
begin
  -- clk: 0 1 H 0 L 1 X 1 0 H U 1 (rising: 0->1, L->1, X->1 no, 0->H, 0->1 after U no)
  stim : process
    type levels is array (natural range <>) of std_logic;
    constant seq : levels := ('1', 'H', '0', 'L', '1', 'X', '1', '0', 'H', 'U', '1', '0', '1');
  begin
    for i in seq'range loop
      wait for 10 ns;
      clk <= seq(i);
      v(2) <= seq(i);
      v(1) <= not seq(i);
      r.a <= seq(i);
      if i = 5 then en <= '1'; end if;
      if i = 9 then en <= '0'; end if;
    end loop;
    wait for 10 ns;
    rst <= '1';
    wait for 10 ns;
    assert c_up = 4 report "c_up " & integer'image(c_up) severity failure;
    assert c_dn = 3 report "c_dn " & integer'image(c_dn) severity failure;
    assert c_en = 1 report "c_en " & integer'image(c_en) severity failure;
    assert c_wu = 4 report "c_wu " & integer'image(c_wu) severity failure;
    assert c_wd = 3 report "c_wd " & integer'image(c_wd) severity failure;
    assert c_wi = 4 report "c_wi " & integer'image(c_wi) severity failure;
    assert c_sl = 4 report "c_sl " & integer'image(c_sl) severity failure;
    assert c_rec = 4 report "c_rec " & integer'image(c_rec) severity failure;
    report "done";
    wait;
  end process;

  up : process (v) begin
    if rising_edge(v(2)) then c_up <= c_up + 1; end if;
  end process;

  dn : process (v) begin
    if falling_edge(v(2)) then c_dn <= c_dn + 1; end if;
  end process;

  en_and : process (clk, en) begin
    if rising_edge(clk) and en = '1' then c_en <= c_en + 1; end if;
  end process;

  wait_until : process begin
    wait until rising_edge(clk);
    c_wu <= c_wu + 1;
  end process;

  wait_on_until : process begin
    wait on clk, v until falling_edge(clk) and v(1) = '1';
    c_wd <= c_wd + 1;
  end process;

  wait_first : process begin
    c_wi <= c_wi + 1;
    wait until rising_edge(v(1));
  end process;

  sens_two : process (clk, rst) begin
    if rising_edge(clk) then c_sl <= c_sl + 1; end if;
  end process;

  field : process (r) begin
    if rising_edge(r.a) then c_rec <= c_rec + 1; end if;
  end process;
end architecture;
