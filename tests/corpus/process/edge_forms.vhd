-- difftest: top=edge_forms std=2008 ref=hand
-- ref=hand: the first reference simulator runs the process of `x'event and x = '0'` as true once at time 0 (the
-- second one and the LRM: no event at initialization); the three `*_lf` counts are edited to the LRM result.
-- Every spelling of a clock edge on std_logic, std_ulogic and bit: rising_edge/falling_edge,
-- `x'event and x = '1'`, `not x'stable and x = '1'`, `wait until x = '1'` and the wait forms.
-- Weak and unknown levels must give the same counts as without any kernel shortcut.
library ieee;
use ieee.std_logic_1164.all;

entity edge_forms is end;

architecture t of edge_forms is
  signal s : std_logic := '0';
  signal u : std_ulogic := '0';
  signal b : bit := '0';
  signal rb : bit := '0';
  signal en : std_logic := '0';
  signal s_re, s_fe, s_lr, s_lf, s_nr, s_nf, s_wr, s_wf, s_wl, s_wo, u_re, u_fe, u_lr, u_lf, u_nr, u_nf, u_wr, u_wf, u_wl, u_wo, b_re, b_fe, b_lr, b_lf, b_nr, b_nf, b_wr, b_wf, b_wl, b_wo, b_rs, s_ea, s_ae, s_ee, s_w1, s_w2, s_w3, u_w1, b_w1 : natural := 0;
begin
  stim : process
    type levels is array (natural range <>) of std_logic;
    constant seq : levels := ('1', 'H', '0', 'L', '1', 'X', '1', '0', 'H', 'U', '1', '0', '1', 'Z', '1', 'W', '0', '-', '1');
    constant bseq : bit_vector(0 to 9) := "1100101101";
  begin
    for i in seq'range loop
      wait for 10 ns;
      s <= seq(i);
      u <= seq(i);
      if i = 4 then en <= '1'; end if;
      if i = 12 then en <= '0'; end if;
    end loop;
    for i in bseq'range loop
      wait for 10 ns;
      b <= bseq(i);
      if i = 3 then rb <= '1'; end if;
      if i = 6 then rb <= '0'; end if;
    end loop;
    wait for 10 ns;
    report "s_re = " & integer'image(s_re);
    report "s_fe = " & integer'image(s_fe);
    report "s_lr = " & integer'image(s_lr);
    report "s_lf = " & integer'image(s_lf);
    report "s_nr = " & integer'image(s_nr);
    report "s_nf = " & integer'image(s_nf);
    report "s_wr = " & integer'image(s_wr);
    report "s_wf = " & integer'image(s_wf);
    report "s_wl = " & integer'image(s_wl);
    report "s_wo = " & integer'image(s_wo);
    report "u_re = " & integer'image(u_re);
    report "u_fe = " & integer'image(u_fe);
    report "u_lr = " & integer'image(u_lr);
    report "u_lf = " & integer'image(u_lf);
    report "u_nr = " & integer'image(u_nr);
    report "u_nf = " & integer'image(u_nf);
    report "u_wr = " & integer'image(u_wr);
    report "u_wf = " & integer'image(u_wf);
    report "u_wl = " & integer'image(u_wl);
    report "u_wo = " & integer'image(u_wo);
    report "b_re = " & integer'image(b_re);
    report "b_fe = " & integer'image(b_fe);
    report "b_lr = " & integer'image(b_lr);
    report "b_lf = " & integer'image(b_lf);
    report "b_nr = " & integer'image(b_nr);
    report "b_nf = " & integer'image(b_nf);
    report "b_wr = " & integer'image(b_wr);
    report "b_wf = " & integer'image(b_wf);
    report "b_wl = " & integer'image(b_wl);
    report "b_wo = " & integer'image(b_wo);
    report "b_rs = " & integer'image(b_rs);
    report "s_ea = " & integer'image(s_ea);
    report "s_ae = " & integer'image(s_ae);
    report "s_ee = " & integer'image(s_ee);
    report "s_w1 = " & integer'image(s_w1);
    report "s_w2 = " & integer'image(s_w2);
    report "s_w3 = " & integer'image(s_w3);
    report "u_w1 = " & integer'image(u_w1);
    report "b_w1 = " & integer'image(b_w1);
    wait;
  end process;

  p_s_re : process (s) begin
    if rising_edge(s) then s_re <= s_re + 1; end if;
  end process;
  p_s_fe : process (s) begin
    if falling_edge(s) then s_fe <= s_fe + 1; end if;
  end process;
  p_s_lr : process (s) begin
    if s'event and s = '1' then s_lr <= s_lr + 1; end if;
  end process;
  p_s_lf : process (s) begin
    if s'event and s = '0' then s_lf <= s_lf + 1; end if;
  end process;
  p_s_nr : process (s) begin
    if not s'stable and s = '1' then s_nr <= s_nr + 1; end if;
  end process;
  p_s_nf : process (s) begin
    if s = '0' and not s'stable then s_nf <= s_nf + 1; end if;
  end process;
  p_s_wr : process begin
    wait until s = '1';
    s_wr <= s_wr + 1;
  end process;
  p_s_wf : process begin
    wait until s = '0';
    s_wf <= s_wf + 1;
  end process;
  p_s_wl : process begin
    wait until s'event and s = '1';
    s_wl <= s_wl + 1;
  end process;
  p_s_wo : process begin
    wait until rising_edge(s);
    s_wo <= s_wo + 1;
  end process;
  p_u_re : process (u) begin
    if rising_edge(u) then u_re <= u_re + 1; end if;
  end process;
  p_u_fe : process (u) begin
    if falling_edge(u) then u_fe <= u_fe + 1; end if;
  end process;
  p_u_lr : process (u) begin
    if u'event and u = '1' then u_lr <= u_lr + 1; end if;
  end process;
  p_u_lf : process (u) begin
    if u'event and u = '0' then u_lf <= u_lf + 1; end if;
  end process;
  p_u_nr : process (u) begin
    if not u'stable and u = '1' then u_nr <= u_nr + 1; end if;
  end process;
  p_u_nf : process (u) begin
    if u = '0' and not u'stable then u_nf <= u_nf + 1; end if;
  end process;
  p_u_wr : process begin
    wait until u = '1';
    u_wr <= u_wr + 1;
  end process;
  p_u_wf : process begin
    wait until u = '0';
    u_wf <= u_wf + 1;
  end process;
  p_u_wl : process begin
    wait until u'event and u = '1';
    u_wl <= u_wl + 1;
  end process;
  p_u_wo : process begin
    wait until rising_edge(u);
    u_wo <= u_wo + 1;
  end process;
  p_b_re : process (b) begin
    if rising_edge(b) then b_re <= b_re + 1; end if;
  end process;
  p_b_fe : process (b) begin
    if falling_edge(b) then b_fe <= b_fe + 1; end if;
  end process;
  p_b_lr : process (b) begin
    if b'event and b = '1' then b_lr <= b_lr + 1; end if;
  end process;
  p_b_lf : process (b) begin
    if b'event and b = '0' then b_lf <= b_lf + 1; end if;
  end process;
  p_b_nr : process (b) begin
    if not b'stable and b = '1' then b_nr <= b_nr + 1; end if;
  end process;
  p_b_nf : process (b) begin
    if b = '0' and not b'stable then b_nf <= b_nf + 1; end if;
  end process;
  p_b_wr : process begin
    wait until b = '1';
    b_wr <= b_wr + 1;
  end process;
  p_b_wf : process begin
    wait until b = '0';
    b_wf <= b_wf + 1;
  end process;
  p_b_wl : process begin
    wait until b'event and b = '1';
    b_wl <= b_wl + 1;
  end process;
  p_b_wo : process begin
    wait until rising_edge(b);
    b_wo <= b_wo + 1;
  end process;
  p_b_rs : process (b, rb) begin
    if rb = '1' then b_rs <= b_rs + 100; elsif rising_edge(b) then b_rs <= b_rs + 1; end if;
  end process;
  p_s_ea : process (s) begin
    if en = '1' and rising_edge(s) then s_ea <= s_ea + 1; end if;
  end process;
  p_s_ae : process (s) begin
    if rising_edge(s) and en = '1' then s_ae <= s_ae + 1; end if;
  end process;
  p_s_ee : process (s, en) begin
    if en = '1' and s = '1' and not s'stable then s_ee <= s_ee + 1; end if;
  end process;
  p_s_w1 : process begin
    wait until rising_edge(s) and en = '1';
    s_w1 <= s_w1 + 1;
  end process;
  p_s_w2 : process begin
    wait on s, en until rising_edge(s);
    s_w2 <= s_w2 + 1;
  end process;
  p_s_w3 : process begin
    wait until en = '1' and rising_edge(s);
    s_w3 <= s_w3 + 1;
  end process;
  p_u_w1 : process begin
    wait until rising_edge(u) and en = '1';
    u_w1 <= u_w1 + 1;
  end process;
  p_b_w1 : process begin
    wait until rising_edge(b) and rb = '1';
    b_w1 <= b_w1 + 1;
  end process;
end architecture;
