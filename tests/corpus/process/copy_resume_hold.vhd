-- difftest: top=copy_resume_hold
-- The hold monitor of #1138: a pipeline of stages in a generate, each with a monitor that waits on
-- its output, an element of an array of records with an unconstrained field, and checks 'active and
-- 'event. The monitors switch to their specialised copies after 1024 activations (about 12 us here);
-- the first activation of the copy must read this stage's element.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package copy_resume_hold_pkg is
  type axis_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector;
    tlast  : std_logic;
  end record;
  type axis_array_t is array (natural range <>) of axis_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.copy_resume_hold_pkg.all;

entity copy_resume_hold is
  generic (N : positive := 10; W : positive := 16);
end entity;

architecture t of copy_resume_hold is
  signal clk  : std_logic := '0';
  signal done : boolean := false;
  signal ch   : axis_array_t(0 to N)(tdata(W - 1 downto 0));
  signal rdy  : std_logic_vector(0 to N);
  signal cnt  : unsigned(15 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when not done;

  -- Source: a new beat every clock while the first stage accepts.
  process (clk) begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      if rdy(0) = '1' then
        ch(0) <= (tvalid => cnt(0) or cnt(1), tdata => std_logic_vector(resize(cnt * cnt, W)), tlast => cnt(3));
      end if;
    end if;
  end process;
  rdy(N) <= cnt(2) or cnt(4);

  g : for i in 0 to N - 1 generate
    signal stalled_q : std_logic := '0';
  begin
    rdy(i) <= '1' when ch(i + 1).tvalid = '0' or rdy(i + 1) = '1' else '0';
    process (clk) begin
      if rising_edge(clk) then
        if ch(i + 1).tvalid = '0' or rdy(i + 1) = '1' then
          ch(i + 1) <= ch(i);
        end if;
      end if;
    end process;
    process (clk) begin
      if rising_edge(clk) then
        if ch(i + 1).tvalid = '1' and rdy(i + 1) = '0' then
          stalled_q <= '1';
        else
          stalled_q <= '0';
        end if;
      end if;
    end process;
    process begin
      wait on ch(i + 1);
      assert ch(i + 1)'active and not (ch(i + 1).tdata'event and stalled_q = '1')
        report "hold violated (event)" severity failure;
    end process;
  end generate;

  process begin
    wait for 20 us;
    done <= true;
    report "end: cnt " & integer'image(to_integer(cnt)) & " last " & to_hstring(ch(N).tdata);
    wait;
  end process;
end architecture;
