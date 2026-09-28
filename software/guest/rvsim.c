/*
 * rvsim: control the rvsim simulator from inside a Linux guest.
 *
 * Writes commands to the simulator's sim-control device through /dev/mem:
 *
 *   rvsim dump-stats LABEL          keep a labelled snapshot of the stats
 *   rvsim reset-stats               zero the stats
 *   rvsim exit [CODE]               end the simulation
 *   rvsim run START END CMD [ARG]   dump START, run CMD, dump END
 *
 * `run` brackets the command as tightly as a separate process allows: the
 * dumps are taken in this process just before the fork and just after the
 * wait, so the region is the command's whole process lifetime.
 */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#define SIM_CONTROL_BASE 0x00102000UL
#define REG_COMMAND 0x00
#define REG_ARG 0x08

enum command { RESET_STATS = 1, DUMP_STATS = 2, EXIT = 3 };

static volatile uint64_t *device;

static void map_device(void)
{
	int fd = open("/dev/mem", O_RDWR | O_SYNC);
	if (fd < 0) {
		perror("rvsim: /dev/mem");
		exit(2);
	}
	void *page = mmap(NULL, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
			  SIM_CONTROL_BASE);
	close(fd);
	if (page == MAP_FAILED) {
		perror("rvsim: mmap");
		exit(2);
	}
	device = page;
}

static void issue(enum command command, uint64_t arg)
{
	device[REG_ARG / 8] = arg;
	device[REG_COMMAND / 8] = command;
}

static uint64_t number(const char *text)
{
	char *end;
	uint64_t value = strtoull(text, &end, 0);
	if (*text == '\0' || *end != '\0') {
		fprintf(stderr, "rvsim: not a number: %s\n", text);
		exit(2);
	}
	return value;
}

static int run(uint64_t start, uint64_t end, char **argv)
{
	issue(DUMP_STATS, start);
	pid_t child = fork();
	if (child < 0) {
		perror("rvsim: fork");
		return 2;
	}
	if (child == 0) {
		execvp(argv[0], argv);
		perror("rvsim: exec");
		_exit(127);
	}
	int status;
	waitpid(child, &status, 0);
	issue(DUMP_STATS, end);
	return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}

static void usage(void)
{
	fputs("usage: rvsim dump-stats LABEL | reset-stats | exit [CODE] |"
	      " run START END CMD [ARG...]\n",
	      stderr);
	exit(2);
}

int main(int argc, char **argv)
{
	if (argc < 2)
		usage();
	map_device();
	const char *verb = argv[1];
	if (strcmp(verb, "dump-stats") == 0 && argc == 3) {
		issue(DUMP_STATS, number(argv[2]));
	} else if (strcmp(verb, "reset-stats") == 0 && argc == 2) {
		issue(RESET_STATS, 0);
	} else if (strcmp(verb, "exit") == 0 && argc <= 3) {
		issue(EXIT, argc == 3 ? number(argv[2]) : 0);
	} else if (strcmp(verb, "run") == 0 && argc >= 5) {
		return run(number(argv[2]), number(argv[3]), argv + 4);
	} else {
		usage();
	}
	return 0;
}
