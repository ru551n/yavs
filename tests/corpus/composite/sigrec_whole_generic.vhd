-- difftest: top=sigrec_whole_generic std=2008
-- A whole record signal whose field widths come from a generic (#677): the layout of the record
-- value differs per instance, and one process code serves instances of 3, 9 and 70 bits. The
-- process reads the record as a whole, changes some fields and drives it back as a whole.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity sigrec_whole_generic_unit is
  generic (W : positive; tag : string);
  port (clk : in std_logic);
end;
architecture a of sigrec_whole_generic_unit is
  type rec is record
    flag : std_logic;
    v : std_logic_vector(W - 1 downto 0);
    n : natural range 0 to 1000;
    u : unsigned(W downto 0);
    m : std_logic_vector(W - 1 downto 0);
    b : boolean;
  end record;
  signal r, rin : rec := (flag => '0', v => (others => '0'), n => 1, u => (others => '0'),
                          m => (0 => '1', others => '0'), b => false);
  signal cnt : natural := 0;
begin
  comb : process (r)
    variable x : rec;
  begin
    x := r;
    x.flag := not r.flag;
    x.v := std_logic_vector(unsigned(r.v) + 3);
    x.n := (r.n * 3 + 1) mod 1000;
    x.u := r.u + 5;
    x.m := r.m(W - 2 downto 0) & r.m(W - 1);
    x.b := not r.b;
    rin <= x;
  end process;

  process (clk) begin
    if rising_edge(clk) then
      r <= rin;
      cnt <= cnt + 1;
    end if;
  end process;

  process (clk)
  begin
    if falling_edge(clk) and cnt mod 4 = 0 and cnt > 0 then
      report tag & " cnt=" & integer'image(cnt) & " flag=" & std_logic'image(r.flag) & " v=" & to_hstring(r.v)
        & " n=" & integer'image(r.n) & " u=" & to_hstring(std_logic_vector(r.u)) & " m=" & to_string(r.m)
        & " b=" & boolean'image(r.b);
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all;
entity sigrec_whole_generic is end;
architecture t of sigrec_whole_generic is
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 100 ns;
  u3 : entity work.sigrec_whole_generic_unit generic map (W => 3, tag => "u3") port map (clk);
  u9 : entity work.sigrec_whole_generic_unit generic map (W => 9, tag => "u9") port map (clk);
  u70 : entity work.sigrec_whole_generic_unit generic map (W => 70, tag => "u70") port map (clk);
end;
