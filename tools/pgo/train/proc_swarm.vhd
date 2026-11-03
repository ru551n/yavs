-- pgo: top=proc_swarm_tb std=2008 stop=8ms
-- A swarm of hundreds of tiny clocked processes built by nested generate loops:
-- counters, shift registers, edge detectors, stretchers, debouncers, toggles,
-- small FSMs, shared-line drivers and delayed registers, with several
-- clocking and reset styles. A model in the testbench checks every lane.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity debounce is
  generic (limit : positive := 3);
  port (
    clk   : in  std_logic;
    rst_n : in  std_logic;
    d     : in  std_logic;
    q     : out std_logic
  );
end entity debounce;

architecture rtl of debounce is
  signal sync : std_logic := '0';
  signal cnt  : natural range 0 to limit := 0;
  signal q_r  : std_logic := '0';
begin
  process (clk, rst_n)
  begin
    if rst_n = '0' then
      sync <= '0';
      cnt  <= 0;
      q_r  <= '0';
    elsif rising_edge(clk) then
      sync <= d;
      if d = sync then
        if cnt = limit then
          q_r <= d;
        else
          cnt <= cnt + 1;
        end if;
      else
        cnt <= 0;
      end if;
    end if;
  end process;
  q <= q_r;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity swarm is
  generic (ngroups : positive := 4);
  port (
    clk      : in  std_logic;
    rst_n    : in  std_logic;
    ce       : in  std_logic;
    stim     : in  std_logic_vector(7 downto 0);
    lane_out : out std_logic_vector(0 to ngroups * 10 - 1)
  );
end entity swarm;

architecture rtl of swarm is
begin
  g_grp : for g in 0 to ngroups - 1 generate
    g_k : for k in 0 to 9 generate
      constant IDX : natural := g * 10 + k;
      constant SI  : natural := IDX mod 8;
    begin
      -- kind 0: counter with enable and synchronous reset
      g0 : if k = 0 generate
        signal cnt : unsigned(3 downto 0) := (others => '0');
      begin
        process (clk)
        begin
          if rising_edge(clk) then
            if rst_n = '0' then
              cnt <= (others => '0');
            elsif ce = '1' then
              cnt <= cnt + 1;
            end if;
          end if;
        end process;
        lane_out(IDX) <= cnt(3);
      end generate g0;

      -- kind 1: shift register, old style clock test
      g1 : if k = 1 generate
        signal sr : std_logic_vector(7 downto 0) := (others => '0');
      begin
        process (clk)
        begin
          if clk'event and clk = '1' then
            sr <= sr(6 downto 0) & stim(SI);
          end if;
        end process;
        lane_out(IDX) <= sr(7) xor sr(0);
      end generate g1;

      -- kind 2: rising edge detector with asynchronous reset
      g2 : if k = 2 generate
        signal prev, pulse : std_logic := '0';
      begin
        process (clk, rst_n)
        begin
          if rst_n = '0' then
            prev  <= '0';
            pulse <= '0';
          elsif rising_edge(clk) then
            pulse <= stim(SI) and not prev;
            prev  <= stim(SI);
          end if;
        end process;
        lane_out(IDX) <= pulse;
      end generate g2;

      -- kind 3: pulse stretcher written with a wait statement
      g3 : if k = 3 generate
        signal hold : natural range 0 to 3 := 0;
      begin
        process
        begin
          wait until rising_edge(clk);
          if stim(SI) = '1' then
            hold <= 3;
          elsif hold /= 0 then
            hold <= hold - 1;
          end if;
        end process;
        lane_out(IDX) <= '1' when hold /= 0 else '0';
      end generate g3;

      -- kind 4: debouncer as an entity
      g4 : if k = 4 generate
      begin
        u_db : entity work.debounce
          generic map (limit => 3)
          port map (clk => clk, rst_n => rst_n, d => stim(SI), q => lane_out(IDX));
      end generate g4;

      -- kind 5: toggle flip-flop, asynchronous reset, old style clock test
      g5 : if k = 5 generate
        signal q : std_logic := '0';
      begin
        process (clk, rst_n)
        begin
          if rst_n = '0' then
            q <= '0';
          elsif clk'event and clk = '1' then
            if stim(SI) = '1' and ce = '1' then
              q <= not q;
            end if;
          end if;
        end process;
        lane_out(IDX) <= q;
      end generate g5;

      -- kind 6: small state machine
      g6 : if k = 6 generate
        type st_t is (IDLE, RUN, DONE);
        signal st  : st_t := IDLE;
        signal cnt : natural range 0 to 3 := 0;
      begin
        process (clk)
        begin
          if rising_edge(clk) then
            if rst_n = '0' then
              st  <= IDLE;
              cnt <= 0;
            else
              case st is
                when IDLE =>
                  if stim(SI) = '1' then
                    st  <= RUN;
                    cnt <= 0;
                  end if;
                when RUN =>
                  if ce = '1' then
                    if cnt = 2 then
                      st <= DONE;
                    else
                      cnt <= cnt + 1;
                    end if;
                  end if;
                when DONE =>
                  if stim(SI) = '0' then
                    st <= IDLE;
                  end if;
              end case;
            end if;
          end if;
        end process;
        lane_out(IDX) <= '1' when st = DONE else '0';
      end generate g6;

      -- kind 7: one resolved line, two drivers (three in odd groups)
      g7 : if k = 7 generate
        signal line : std_logic;
      begin
        pa : process (clk)
        begin
          if rising_edge(clk) then
            if stim(SI) = '1' and ce = '1' then
              line <= '1';
            else
              line <= 'Z';
            end if;
          end if;
        end process;
        pb : process (clk)
        begin
          if rising_edge(clk) then
            if stim(SI) = '0' then
              line <= '0';
            else
              line <= 'Z';
            end if;
          end if;
        end process;
        g_c : if g mod 2 = 1 generate
        begin
          pc : process (clk)
          begin
            if rising_edge(clk) then
              if stim(SI) = '1' and ce = '1' then
                line <= '0';
              else
                line <= 'Z';
              end if;
            end if;
          end process;
        end generate g_c;
        lane_out(IDX) <= line;
      end generate g7;

      -- kind 8: two registers with transport style delays on the assignments
      g8 : if k = 8 generate
        signal q1, q2 : std_logic := '0';
      begin
        process (clk)
        begin
          if rising_edge(clk) then
            q1 <= stim(SI) after 1 ns;
            q2 <= q1 after 2 ns;
          end if;
        end process;
        lane_out(IDX) <= q2 after 500 ps;
      end generate g8;

      -- kind 9: parity accumulator with enable, wait on a clock event
      g9 : if k = 9 generate
        signal acc : std_logic := '0';
      begin
        process
        begin
          wait until clk'event and clk = '1';
          if ce = '1' then
            acc <= acc xor stim(SI);
          end if;
        end process;
        lane_out(IDX) <= acc;
      end generate g9;
    end generate g_k;
  end generate g_grp;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity proc_swarm_tb is
end entity proc_swarm_tb;

architecture sim of proc_swarm_tb is
  constant NG    : positive := 40;
  constant NL    : positive := NG * 10;
  constant N_CYC : positive := 5000;

  signal clk   : std_logic := '0';
  signal rst_n : std_logic := '0';
  signal ce    : std_logic := '0';
  signal stim  : std_logic_vector(7 downto 0) := (others => '0');
  signal lane_out : std_logic_vector(0 to NL - 1);
  signal done  : boolean := false;
  signal cycle : natural := 0;

  function to_sl (b : boolean) return std_logic is
  begin
    if b then
      return '1';
    end if;
    return '0';
  end function;

  function xorshift (s : unsigned(31 downto 0)) return unsigned is
    variable x : unsigned(31 downto 0) := s;
  begin
    x := x xor shift_left(x, 13);
    x := x xor shift_right(x, 17);
    x := x xor shift_left(x, 5);
    return x;
  end function;

  -- Spread a lane vector into one 32-bit word.
  function fold (v : std_logic_vector) return unsigned is
    constant K : unsigned(31 downto 0) := x"9E3779B9";
    variable r : unsigned(31 downto 0) := (others => '0');
  begin
    for i in v'range loop
      if to_x01(v(i)) = '1' then
        r := r xor rotate_left(K, i mod 32);
      elsif to_x01(v(i)) = 'X' then
        r := r xor shift_right(K, 1 + i mod 7);
      end if;
    end loop;
    return r;
  end function;
begin
  clk <= not clk after 5 ns when not done else clk;

  dut : entity work.swarm
    generic map (ngroups => NG)
    port map (clk => clk, rst_n => rst_n, ce => ce, stim => stim, lane_out => lane_out);

  -- Sticky random stimulus, clock enable and occasional resets.
  stim_p : process (clk)
    variable r    : unsigned(31 downto 0) := x"DEADBEEF";
    variable sv   : std_logic_vector(7 downto 0) := (others => '0');
    variable cyc  : natural := 0;
  begin
    if rising_edge(clk) then
      cyc := cyc + 1;
      cycle <= cyc;
      r := xorshift(r);
      for i in 0 to 7 loop
        if r(2 * i) = '1' and r(2 * i + 1) = '1' then
          sv(i) := not sv(i);
        end if;
      end loop;
      stim <= sv after 2 ns;
      if cyc mod 3 = 2 then
        ce <= '0' after 2 ns;
      else
        ce <= '1' after 2 ns;
      end if;
      if cyc < 5 or (cyc mod 1500 >= 700 and cyc mod 1500 < 703) then
        rst_n <= '0' after 2 ns;
      else
        rst_n <= '1' after 2 ns;
      end if;
    end if;
  end process;

  -- Model of every lane, compared with the design at each clock edge.
  check : process (clk)
    type int_arr is array (0 to NL - 1) of integer;
    type act_arr is array (0 to 9) of natural;
    variable a1, a2, a3 : int_arr := (others => 0);
    variable dA, dB, dC : std_logic_vector(0 to NL - 1) := (others => 'U');
    variable mo : std_logic_vector(0 to NL - 1);
    variable act : act_arr := (others => 0);
    variable cs_dut, cs_mod : unsigned(31 downto 0) := (others => '0');
    variable inited : boolean := false;
    variable d, kind : natural;
    variable c, rst : boolean;
    variable cyc, n_x, n_rst : natural := 0;
  begin
    if rising_edge(clk) and not done then
      if not inited then
        for i in 0 to NL - 1 loop
          if i mod 10 = 7 and (i / 10) mod 2 = 0 then
            dC(i) := 'Z';
          end if;
        end loop;
        inited := true;
      end if;
      cyc := cyc + 1;
      rst := rst_n = '0';
      c := ce = '1';
      if rst then
        n_rst := n_rst + 1;
      end if;

      -- asynchronous reset acts before the edge
      if rst then
        for i in 0 to NL - 1 loop
          kind := i mod 10;
          if kind = 2 or kind = 4 or kind = 5 then
            a1(i) := 0;
            a2(i) := 0;
            a3(i) := 0;
          end if;
        end loop;
      end if;

      -- expected outputs
      for i in 0 to NL - 1 loop
        case i mod 10 is
          when 0 => mo(i) := to_sl(a1(i) >= 8);
          when 1 => mo(i) := to_sl(((a1(i) / 128) mod 2) /= (a1(i) mod 2));
          when 2 => mo(i) := to_sl(a2(i) = 1);
          when 3 => mo(i) := to_sl(a1(i) /= 0);
          when 4 => mo(i) := to_sl(a3(i) = 1);
          when 5 => mo(i) := to_sl(a1(i) = 1);
          when 6 => mo(i) := to_sl(a1(i) = 2);
          when 7 => mo(i) := resolved((dA(i), dB(i), dC(i)));
          when 8 => mo(i) := to_sl(a2(i) = 1);
          when others => mo(i) := to_sl(a1(i) = 1);
        end case;
        if mo(i) = '1' then
          act(i mod 10) := act(i mod 10) + 1;
        elsif mo(i) = 'X' then
          n_x := n_x + 1;
        end if;
        assert lane_out(i) = mo(i)
          report "lane " & integer'image(i) & " kind " & integer'image(i mod 10)
                 & " cycle " & integer'image(cyc) & ": design " & std_logic'image(lane_out(i))
                 & " model " & std_logic'image(mo(i))
          severity failure;
      end loop;
      cs_dut := rotate_left(cs_dut, 5) xor fold(lane_out);
      cs_mod := rotate_left(cs_mod, 5) xor fold(mo);

      -- state update with the values sampled at this edge
      for i in 0 to NL - 1 loop
        d := 0;
        if stim(i mod 8) = '1' then
          d := 1;
        end if;
        case i mod 10 is
          when 0 =>
            if rst then
              a1(i) := 0;
            elsif c then
              a1(i) := (a1(i) + 1) mod 16;
            end if;
          when 1 =>
            a1(i) := (a1(i) * 2) mod 256 + d;
          when 2 =>
            if not rst then
              if d = 1 and a1(i) = 0 then
                a2(i) := 1;
              else
                a2(i) := 0;
              end if;
              a1(i) := d;
            end if;
          when 3 =>
            if d = 1 then
              a1(i) := 3;
            elsif a1(i) /= 0 then
              a1(i) := a1(i) - 1;
            end if;
          when 4 =>
            if not rst then
              if d = a1(i) then
                if a2(i) = 3 then
                  a3(i) := d;
                else
                  a2(i) := a2(i) + 1;
                end if;
              else
                a2(i) := 0;
              end if;
              a1(i) := d;
            end if;
          when 5 =>
            if not rst and d = 1 and c then
              a1(i) := 1 - a1(i);
            end if;
          when 6 =>
            if rst then
              a1(i) := 0;
              a2(i) := 0;
            elsif a1(i) = 0 then
              if d = 1 then
                a1(i) := 1;
                a2(i) := 0;
              end if;
            elsif a1(i) = 1 then
              if c then
                if a2(i) = 2 then
                  a1(i) := 2;
                else
                  a2(i) := a2(i) + 1;
                end if;
              end if;
            elsif d = 0 then
              a1(i) := 0;
            end if;
          when 7 =>
            if d = 1 and c then
              dA(i) := '1';
            else
              dA(i) := 'Z';
            end if;
            if d = 0 then
              dB(i) := '0';
            else
              dB(i) := 'Z';
            end if;
            if (i / 10) mod 2 = 1 then
              if d = 1 and c then
                dC(i) := '0';
              else
                dC(i) := 'Z';
              end if;
            end if;
          when 8 =>
            a2(i) := a1(i);
            a1(i) := d;
          when others =>
            if c then
              a1(i) := (a1(i) + d) mod 2;
            end if;
        end case;
      end loop;

      if cyc = N_CYC then
        assert cs_dut = cs_mod report "checksum mismatch" severity failure;
        for k in 0 to 9 loop
          assert act(k) > N_CYC / 50
            report "kind " & integer'image(k) & " barely active: " & integer'image(act(k))
            severity failure;
        end loop;
        assert n_x > 0 report "no conflicting driver seen" severity failure;
        assert n_rst > 10 report "reset schedule not run" severity failure;
        report "lanes=" & integer'image(NL) & " cycles=" & integer'image(cyc)
               & " checksum=" & to_hstring(cs_dut) & " conflicts=" & integer'image(n_x);
        report "PASS" severity note;
        done <= true;
      end if;
    end if;
  end process check;
end architecture sim;
