# up veth pair
sudo ip link del sender 2>/dev/null || true
sudo ip link add dev sender type veth peer name receiver
sudo ip link set sender up
sudo ip link set receiver up
sudo ip link set sender promisc on
sudo ip link set receiver promisc on
sudo ip link set dev sender address 02:00:00:00:00:01
sudo ip link set dev receiver address 02:00:00:00:00:02