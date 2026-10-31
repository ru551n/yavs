-- difftest: top=protected_method_elab std=2008
-- A method call of a protected shared variable during elaboration, in declaration order: in a
-- constant that bounds a signal, a generic map and a generate, of a region's and a package's
-- variable (#912).
package pk_me is
  type rnd_t is protected
    impure function next_val(lo, hi : integer) return integer;
  end protected;
end;
package body pk_me is
  type rnd_t is protected body
    variable n : integer := 0;
    impure function next_val(lo, hi : integer) return integer is
    begin
      n := n + 1;
      return lo + n mod (hi - lo + 1);
    end function;
  end protected body;
end;
package sv_me is
  use work.pk_me.all;
  shared variable prnd : rnd_t;
end;
use work.pk_me.all;
use work.sv_me.all;
entity sub_me is generic (g : positive); end;
architecture a of sub_me is
  shared variable lrnd : rnd_t;
  constant l : positive := lrnd.next_val(g, g + 1);
  signal x : bit_vector(l - 1 downto 0);
begin
  process begin report "sub_me g=" & integer'image(g) & " l=" & integer'image(x'length); wait; end process;
end;
use work.pk_me.all;
use work.sv_me.all;
entity protected_method_elab is end entity;
architecture a of protected_method_elab is
  shared variable rnd : rnd_t;
  constant w1 : positive := 8;
  constant w2 : positive := rnd.next_val(w1 - 4, w1);
  signal s : bit_vector(w2 - 1 downto 0) := (others => '0');
  constant w3 : positive := maximum(1, rnd.next_val(w1 - 4, w1));
  signal s3 : bit_vector(1 + rnd.next_val(1, 4) downto 0);
  constant w4 : positive := prnd.next_val(2, 9);
  signal s4 : bit_vector(w4 downto 1);
  constant w5 : positive := w2 + rnd.next_val(1, 2);
begin
  u1 : entity work.sub_me generic map (g => rnd.next_val(1, 5));
  g1 : for i in 1 to 2 generate
    constant c : positive := rnd.next_val(i, 5);
    signal t : bit_vector(c downto 1);
  begin
    u : entity work.sub_me generic map (g => c);
    process begin report integer'image(i) & " c=" & integer'image(t'length); wait; end process;
  end generate;
  process begin
    report "w2 = " & integer'image(w2) & " s'length = " & integer'image(s'length) & " w3=" & integer'image(w3)
      & " s3=" & integer'image(s3'length) & " s4=" & integer'image(s4'length) & " w5=" & integer'image(w5);
    report integer'image(rnd.next_val(0, 1000));
    wait;
  end process;
end architecture;
