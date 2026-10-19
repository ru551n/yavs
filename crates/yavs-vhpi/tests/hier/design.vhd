-- The design the hierarchy plugin (tests/plugin/hier.c) walks: nested component and entity
-- instances, for, if and case generates, a block, a process with a variable, records,
-- multidimensional and enumeration-indexed arrays, an extended identifier and a package.
package Types_Pkg is
  type Rec_T is record
    A : bit;
    B : bit_vector(1 downto 0);
  end record;
  constant Pkg_C : integer := 7;
end package;

use work.Types_Pkg.all;
entity Leaf is
  generic (W : natural := 2; Init : bit_vector := "01");
  port (Clk : in bit; D : in Rec_T; Q : out bit_vector(W - 1 downto 0));
end entity;

architecture Rtl of Leaf is
  signal Reg : bit_vector(W - 1 downto 0);
begin
  Q <= Reg;
  P_Main : process (Clk)
    variable Cnt : integer := 0;
  begin
    if Clk = '1' then
      Cnt := Cnt + 1;
      Reg <= D.B;
    end if;
  end process;
end architecture;

use work.Types_Pkg.all;
entity Top is
end entity;

architecture A of Top is
  component Leaf is
    generic (W : natural);
    port (Clk : in bit; D : in Rec_T; Q : out bit_vector(W - 1 downto 0));
  end component;
  type State_T is (Idle, Run);
  type Counts_T is array (State_T) of integer;
  type Mat_T is array (0 to 1, 1 downto 0) of bit;
  type Arr_T is array (0 to 1) of bit_vector(1 downto 0);
  constant N : integer := 2;
  constant M : Mat_T := (others => (others => '0'));
  signal Clk : bit;
  signal R : Rec_T;
  signal Outs : Arr_T;
  signal Counts : Counts_T;
  signal \Ext Sig\ : bit;
begin
  G : for I in 0 to N - 1 generate
    signal Local : bit;
  begin
    U : Leaf generic map (W => 2) port map (Clk, R, Outs(I));
  end generate;
  B1 : block
    signal Inner : bit;
  begin
  end block;
  IG : if N > 1 generate
    E : entity work.Leaf port map (Clk => Clk, D => R, Q => open);
  end generate;
  CG : case N generate
    when 2 =>
      signal X : bit;
    begin
    end;
    when others =>
  end generate;
end architecture;
