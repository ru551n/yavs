-- difftest: top=constant_reads_signal std=2008 ignore=order
-- ignore=order: reports of different processes in the same cycle.
-- A constant whose initial value reads a signal or an in port takes its initial value
-- while the declarative part is elaborated (LRM 14.4.2.2, 14.5.2; #453).
entity leaf is
  port (p : in bit_vector(3 downto 0); q : in integer);
end;
architecture a of leaf is
  signal local : integer := 5;
  constant cp : bit_vector(3 downto 0) := p;
  constant cq : integer := q * 2 + local;
begin
  process begin
    report "leaf " & to_string(cp) & " " & integer'image(cq);
    wait;
  end process;
end;
entity constant_reads_signal is end;
architecture t of constant_reads_signal is
  type rec is record a : integer; b : bit; end record;
  signal s : bit_vector(3 downto 0) := "1010";
  signal n : integer := 7;
  signal r : rec := (a => 3, b => '1');
  constant c : bit_vector(3 downto 0) := s;
  constant d : integer := n + 1;
  constant e : rec := r;
  constant f : integer := e.a + d;
begin
  u : entity work.leaf port map (p => s, q => n);
  process begin
    report to_string(c) & " " & integer'image(d) & " " & integer'image(e.a) & " " & integer'image(f);
    wait for 1 ns;
    s <= "0101"; n <= 100;
    wait for 1 ns;
    report to_string(c) & " " & integer'image(d);
    wait;
  end process;
  b : block
    constant g : integer := n * 3;
    signal bs : integer := g;
  begin
    process begin
      report "block " & integer'image(g) & " " & integer'image(bs);
      wait;
    end process;
  end block;
end;
