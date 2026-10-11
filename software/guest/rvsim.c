/*
 * rvsim: control the rvsim simulator from inside a Linux guest, with the
 * pseudo-instructions of software/include/rvsim.h (gem5's m5ops), as
 * gem5's m5 tool does:
 *
 *   rvsim dump-stats LABEL          keep a labelled snapshot of the stats
 *   rvsim reset-stats               zero the stats
 *   rvsim exit [CODE]               end the simulation
 *   rvsim break LABEL               stop the host's run here
 *   rvsim run START END CMD [ARG]   dump START, run CMD, dump END
 *   rvsim readfile                  copy the host's script file to stdout
 *   rvsim writefile FILE [NAME]     copy FILE to NAME in the host's output
 *                                   directory (FILE's name by default)
 *
 * `run` brackets the command as tightly as a separate process allows: the
 * dumps are taken in this process just before the fork and just after the
 * wait, so the region is the command's whole process lifetime.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "rvsim.h"

/* Static, and touched before use, so that every page of it is mapped when
 * the simulator reaches it: the pseudo-instruction does not fault pages in. */
static char buffer[256 * 1024];

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
	rvsim_dump_stats(start);
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
	rvsim_dump_stats(end);
	return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}

static void write_all(int fd, const char *data, size_t len)
{
	while (len > 0) {
		ssize_t written = write(fd, data, len);
		if (written <= 0) {
			perror("rvsim: write");
			exit(2);
		}
		data += written;
		len -= (size_t)written;
	}
}

static int read_file(void)
{
	memset(buffer, 0, sizeof(buffer));
	unsigned long offset = 0;
	for (;;) {
		long got = rvsim_read_file(buffer, sizeof(buffer), offset);
		if (got < 0) {
			fprintf(stderr, "rvsim: readfile: %s\n", strerror((int)-got));
			return 2;
		}
		if (got == 0)
			return 0;
		write_all(STDOUT_FILENO, buffer, (size_t)got);
		offset += (unsigned long)got;
	}
}

static int write_file(const char *path, const char *name)
{
	FILE *source = fopen(path, "rb");
	if (source == NULL) {
		fprintf(stderr, "rvsim: %s: %s\n", path, strerror(errno));
		return 2;
	}
	memset(buffer, 0, sizeof(buffer));
	unsigned long offset = 0;
	for (;;) {
		size_t got = fread(buffer, 1, sizeof(buffer), source);
		if (got == 0) {
			if (ferror(source)) {
				fprintf(stderr, "rvsim: %s: %s\n", path, strerror(errno));
				fclose(source);
				return 2;
			}
			break;
		}
		long written = rvsim_write_file(buffer, got, offset, name);
		if (written < 0) {
			fprintf(stderr, "rvsim: writefile %s: %s\n", name, strerror((int)-written));
			fclose(source);
			return 2;
		}
		if ((size_t)written != got) {
			fprintf(stderr, "rvsim: writefile %s: short write\n", name);
			fclose(source);
			return 2;
		}
		offset += (unsigned long)written;
	}
	fclose(source);
	if (offset == 0 && rvsim_write_file(buffer, 0, 0, name) < 0) {
		fprintf(stderr, "rvsim: writefile %s: could not create it\n", name);
		return 2;
	}
	return 0;
}

static void usage(void)
{
	fputs("usage: rvsim dump-stats LABEL | reset-stats | break LABEL | exit [CODE] |"
	      " run START END CMD [ARG...] | readfile | writefile FILE [NAME]\n",
	      stderr);
	exit(2);
}

int main(int argc, char **argv)
{
	if (argc < 2)
		usage();
	const char *verb = argv[1];
	if (strcmp(verb, "dump-stats") == 0 && argc == 3) {
		rvsim_dump_stats(number(argv[2]));
	} else if (strcmp(verb, "reset-stats") == 0 && argc == 2) {
		rvsim_reset_stats();
	} else if (strcmp(verb, "break") == 0 && argc == 3) {
		rvsim_break(number(argv[2]));
	} else if (strcmp(verb, "exit") == 0 && argc <= 3) {
		rvsim_exit(argc == 3 ? number(argv[2]) : 0);
	} else if (strcmp(verb, "run") == 0 && argc >= 5) {
		return run(number(argv[2]), number(argv[3]), argv + 4);
	} else if (strcmp(verb, "readfile") == 0 && argc == 2) {
		return read_file();
	} else if (strcmp(verb, "writefile") == 0 && (argc == 3 || argc == 4)) {
		return write_file(argv[2], argc == 4 ? argv[3] : argv[2]);
	} else {
		usage();
	}
	return 0;
}
