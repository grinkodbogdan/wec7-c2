// C++ runtime smoke test: exceptions, iostreams, threads, containers.
#include <iostream>
#include <map>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

static int depth(int n) {
	if (n == 0)
		throw std::runtime_error("thrown from deep inside");
	return depth(n - 1) + 1;
}

int main() {
	int fails = 0;
	try {
		depth(20);
		fails++;
	} catch (const std::exception &e) {
		std::cout << "  ok     exception caught: " << e.what() << "\n";
	}
	std::map<std::string, int> m{{"dragonfly", 1}, {"arm64", 2}};
	std::ostringstream os;
	os << m["dragonfly"] + m["arm64"] << " " << 3.25;
	std::cout << "  " << (os.str() == "3 3.25" ? "ok    " : "FAILED") << " map, ostringstream: " << os.str() << "\n";
	fails += os.str() != "3 3.25";
	std::mutex mu;
	long total = 0;
	std::vector<std::thread> ts;
	for (int i = 0; i < 4; i++)
		ts.emplace_back([&, i] {
			for (int j = 0; j < 100000; j++) {
				std::lock_guard<std::mutex> g(mu);
				total += i;
			}
		});
	for (auto &t : ts)
		t.join();
	std::cout << "  " << (total == 600000 ? "ok    " : "FAILED") << " std::thread x4, total " << total << "\n";
	fails += total != 600000;
	std::cout << (fails ? "c++: SOME CHECKS FAILED" : "c++: all checks passed") << std::endl;
	return fails;
}
