-- A process like a memory model: an array of access values sized by a generic, a nested
-- procedure that indexes it and writes a variable of generic width, and a signal driven from that
-- variable. The copy folds the variable's length check (#1063); the checksum must not depend on it.
entity spec_nested_mem_u is
  generic (AB : positive := 3; W : positive := 8);
  port (clk : in bit; d : in bit_vector(W - 1 downto 0); q : out bit_vector(W - 1 downto 0));
end;
architecture a of spec_nested_mem_u is
begin
  process
    type row is access bit_vector;
    type rows is array (2 ** AB - 1 downto 0) of row;
    variable bank : rows;
    variable dq : bit_vector(W - 1 downto 0) := (others => '0');
    variable at : natural := 0;
    procedure put(i : natural; x : bit_vector) is
    begin
      if bank(i) = null then
        bank(i) := new bit_vector(W - 1 downto 0);
      end if;
      bank(i).all := x;
      if bank((i + 3) mod 2 ** AB) /= null then
        dq := bank((i + 3) mod 2 ** AB).all xor dq;
      end if;
    end procedure;
  begin
    wait until clk = '1';
    put(at, d);
    at := (at + 1) mod 2 ** AB;
    q <= dq;
  end process;
end;

entity spec_nested_mem is end;
architecture a of spec_nested_mem is
  signal clk : bit := '0';
  signal done : boolean := false;
  signal d8, q8 : bit_vector(7 downto 0) := x"A5";
  signal d5, q5 : bit_vector(4 downto 0) := "10110";
  signal cyc : natural := 0;
begin
  clk <= not clk after 5 ns when not done;
  u8: entity work.spec_nested_mem_u generic map (3, 8) port map (clk, d8, q8);
  u5: entity work.spec_nested_mem_u generic map (2, 5) port map (clk, d5, q5);
  process (clk)
  begin
    if clk = '1' then
      cyc <= cyc + 1;
      d8 <= d8(6 downto 0) & (d8(7) xor d8(2));
      d5 <= d5(3 downto 0) & (d5(4) xor d5(1));
      if cyc = 300 then
        report "sum=" & to_hstring(q8) & " " & to_hstring(q5);
        done <= true;
      end if;
    end if;
  end process;
end;
