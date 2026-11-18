-- A protected method with a loop, too big to inline into compiled code: under --tier=auto the
-- interpreter takes it through an arm of its caller (`opt/inline/arms.rs`), and `--profile`
-- books the line samples of its loops to the method's own lines.
package parms_pkg is
  type store_t is protected
    procedure init(n : natural);
    impure function window(base : natural) return integer;
  end protected;
  procedure init(n : natural);
  impure function total(n : natural) return integer;
end package;
package body parms_pkg is
  type int_vec is array (natural range <>) of integer;
  type int_vec_ptr is access int_vec;

  type store_t is protected body
    variable data : int_vec_ptr;

    procedure init(n : natural) is
    begin
      data := new int_vec'(0 to n - 1 => 3);
    end;

    impure function window(base : natural) return integer is
      variable acc : integer := 0;
    begin
      for j in 0 to 3 loop
        acc := acc + data((base + j) mod 64) * (j + 1);
      end loop;
      for j in 0 to 2 loop
        acc := acc - data((base + 2 * j + 1) mod 64) / (j + 1);
      end loop;
      return acc;
    end;
  end protected body;

  shared variable st : store_t;

  procedure init(n : natural) is
  begin
    st.init(n);
  end;

  -- (Its local array keeps it a call of its own, so it is the function with the arm.)
  impure function total(n : natural) return integer is
    variable acc  : integer := 0;
    variable last : integer_vector(0 to 1) := (others => 0);
  begin
    for i in 0 to n - 1 loop
      last(i mod 2) := st.window(i);
      acc := acc + last(i mod 2);
    end loop;
    return acc + last(0);
  end;
end package body;

use work.parms_pkg.all;
entity profile_arms is
  generic (rounds : natural := 4000);
end;
architecture a of profile_arms is
begin
  process
    variable acc : integer := 0;
  begin
    init(64);
    for k in 1 to rounds loop
      acc := acc + total(64);
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
