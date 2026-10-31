-- Replacement for Microwatt's sim_jtag_socket.vhdl in the realbench harness: no JTAG client is ever connected, so a poll finds no message.
library ieee;
use ieee.std_logic_1164.all;

package sim_jtag_socket is
    procedure sim_jtag_read_msg(out_msg  : out std_ulogic_vector(247 downto 0);
                                out_size : out std_ulogic_vector(7 downto 0));
    procedure sim_jtag_write_msg(in_msg  : in std_ulogic_vector(247 downto 0);
                                 in_size : in std_ulogic_vector(7 downto 0));
end sim_jtag_socket;

package body sim_jtag_socket is
    procedure sim_jtag_read_msg(out_msg  : out std_ulogic_vector(247 downto 0);
                                out_size : out std_ulogic_vector(7 downto 0)) is
    begin
        out_msg := (others => '0');
        out_size := (others => '0');
    end procedure;

    procedure sim_jtag_write_msg(in_msg  : in std_ulogic_vector(247 downto 0);
                                 in_size : in std_ulogic_vector(7 downto 0)) is
    begin
        null;
    end procedure;
end sim_jtag_socket;
